use super::*;
use crate::mcp::Backend;
use anicraft_engine::Session;
use std::sync::atomic::AtomicBool;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A control port on a free port, with a stand-in for the app's frame loop running each call on
/// its own session (as `App::poll_remote` does). Stops when dropped.
struct App {
    port: u16,
    stop: Arc<AtomicBool>,
    loop_thread: Option<std::thread::JoinHandle<Session>>,
}

impl App {
    fn start(session: Session) -> App {
        let server = listen(0, TOKEN.into(), Arc::new(|| {})).unwrap();
        let port = server.port;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let loop_thread = std::thread::spawn(move || {
            let mut s = session;
            while !stop2.load(Ordering::SeqCst) {
                while let Some(r) = server.try_recv() {
                    r.run(&mut s);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            s
        });
        App { port, stop, loop_thread: Some(loop_thread) }
    }

    fn session(mut self) -> Session {
        self.stop.store(true, Ordering::SeqCst);
        self.loop_thread.take().unwrap().join().unwrap()
    }

    fn connect(&self) -> (BufReader<TcpStream>, TcpStream) {
        let s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        (BufReader::new(s.try_clone().unwrap()), s)
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn send(conn: &mut (BufReader<TcpStream>, TcpStream), line: &str) -> Value {
    writeln!(conn.1, "{line}").unwrap();
    serde_json::from_str(&read_line(&mut conn.0, MAX_LINE).unwrap().unwrap()).unwrap()
}

/// The app closed the connection: nothing more to read.
fn closed(conn: &mut (BufReader<TcpStream>, TcpStream)) -> bool {
    matches!(read_line(&mut conn.0, MAX_LINE), Ok(None) | Err(LineError::Io(_)))
}

#[test]
fn requests_are_json_objects_with_a_string_method() {
    let r = parse_request(r#"{"id":3,"token":"t","method":"hello"}"#).unwrap();
    assert_eq!((r.id, r.token.as_deref(), r.method.as_str(), r.params), (json!(3), Some("t"), "hello", json!({})));
    // A web page's fetch to localhost starts with an HTTP request line: refused.
    assert!(parse_request("GET / HTTP/1.1").is_err());
    assert!(parse_request("POST /tools HTTP/1.1").is_err());
    assert!(parse_request("[1,2]").is_err());
    assert!(parse_request(r#"{"id":1}"#).is_err(), "no method");
    assert!(parse_request(r#"{"id":1,"method":7}"#).is_err(), "method must be a string");
}

#[test]
fn tokens_must_match_exactly() {
    assert!(token_matches(Some(TOKEN), TOKEN));
    assert!(!token_matches(None, TOKEN));
    assert!(!token_matches(Some(""), ""), "an empty token never opens the port");
    assert!(!token_matches(Some(&TOKEN[..63]), TOKEN));
    assert!(!token_matches(Some(&TOKEN.replace('f', "e")), TOKEN));
    let t = new_token().unwrap();
    assert_eq!(t.len(), 64);
    assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(t, new_token().unwrap(), "fresh each time");
}

#[test]
fn lines_are_capped() {
    let mut r = &b"one\r\ntwo\nthree"[..];
    assert_eq!(read_line(&mut r, 10).unwrap().as_deref(), Some("one"));
    assert_eq!(read_line(&mut r, 10).unwrap().as_deref(), Some("two"));
    assert_eq!(read_line(&mut r, 10).unwrap().as_deref(), Some("three"));
    assert!(read_line(&mut r, 10).unwrap().is_none());
    let mut exact = &b"0123456789\n"[..];
    assert_eq!(read_line(&mut exact, 10).unwrap().as_deref(), Some("0123456789"));
    let mut long = &b"0123456789A\n"[..];
    assert!(matches!(read_line(&mut long, 10), Err(LineError::TooLong)));
}

#[test]
fn ports_and_bridge_targets() {
    assert_eq!(parse_port("9878"), Ok(9878));
    assert!(parse_port("0").is_err());
    assert!(parse_port("70000").is_err());
    assert!(parse_port("port").is_err());
    let local = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9878));
    assert_eq!(parse_bridge_target("9878"), Ok(local));
    assert_eq!(parse_bridge_target("127.0.0.1:9878"), Ok(local));
    assert_eq!(parse_bridge_target("localhost:9878"), Ok(local));
    assert!(parse_bridge_target("192.168.1.5:9878").is_err(), "this computer only");
    assert!(parse_bridge_target("example.com:9878").is_err());
}

#[test]
fn the_control_file_round_trips() {
    let c = ControlFile { port: 9878, token: TOKEN.into(), pid: 42, version: "0.2.1".into() };
    assert_eq!(ControlFile::parse(&c.to_json().to_string()), Ok(c));
    assert!(ControlFile::parse("{}").is_err());
    assert!(ControlFile::parse("nonsense").is_err());
}

#[test]
fn the_token_comes_from_the_environment_then_the_control_file() {
    let dir = std::env::temp_dir().join(format!("anicraft-control-token-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(CONTROL_FILE);
    std::fs::write(&file, ControlFile { port: 9878, token: "from-file".into(), pid: 1, version: String::new() }.to_json().to_string()).unwrap();
    assert_eq!(find_token(Some("from-env".into()), Some(&file)), Ok("from-env".into()), "the environment wins");
    assert_eq!(find_token(None, Some(&file)), Ok("from-file".into()));
    let missing = find_token(None, Some(&dir.join("gone.json"))).unwrap_err();
    assert!(missing.contains("anicraft --control"), "{missing}");
    assert!(find_token(None, None).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_or_wrong_token_is_refused_and_the_connection_closed() {
    let app = App::start(Session::default());
    let mut c = app.connect();
    let r = send(&mut c, r#"{"id":1,"method":"tools.call","params":{"name":"anicraft_run","arguments":{"command":"draw.rect","params":{"x":0,"y":0,"w":5,"h":5}}}}"#);
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().starts_with("missing token"), "{r}");
    assert!(closed(&mut c));
    let mut c = app.connect();
    let r = send(&mut c, &json!({"id": 2, "token": "nope", "method": "tools.call", "params": {"name": "anicraft_run", "arguments": {"command": "draw.rect", "params": {"x": 0, "y": 0, "w": 5, "h": 5}}}}).to_string());
    assert!(r["error"].as_str().unwrap().starts_with("wrong token"), "{r}");
    assert!(closed(&mut c));
    assert!(app.session().visible_elements().is_empty(), "nothing ran");
}

#[test]
fn http_and_oversized_lines_are_refused_and_the_connection_closed() {
    let app = App::start(Session::default());
    let mut c = app.connect();
    let r = send(&mut c, "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    assert_eq!((r["ok"].clone(), r["id"].clone()), (json!(false), Value::Null));
    assert!(closed(&mut c));
    let mut c = app.connect();
    let big = format!(r#"{{"id":1,"token":"{TOKEN}","method":"hello","pad":"{}"}}"#, "x".repeat(MAX_LINE));
    // The app stops reading at the limit and answers; the rest of the write may fail.
    let _ = c.1.write_all(big.as_bytes());
    let _ = c.1.write_all(b"\n");
    let reply: Value = serde_json::from_str(&read_line(&mut c.0, MAX_LINE).unwrap().unwrap()).unwrap();
    assert!(reply["error"].as_str().unwrap().contains("longer than 4 MiB"), "{reply}");
    assert!(closed(&mut c));
}

#[test]
fn a_call_through_the_port_matches_the_headless_server() {
    let app = App::start(Session::default());
    let mut c = app.connect();
    let hello = send(&mut c, &json!({"id": 1, "token": TOKEN, "method": "hello"}).to_string());
    assert_eq!(hello, json!({"id": 1, "ok": true, "result": {"app": "AniCraft", "version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL}}));
    let list = send(&mut c, &json!({"id": 2, "token": TOKEN, "method": "tools.list"}).to_string());
    assert_eq!(list["result"], mcp::tools());
    let args = json!({"command": "draw.rect", "params": {"x": 10, "y": 10, "w": 50, "h": 40, "fill": "#ff0000"}});
    let live = send(&mut c, &json!({"id": 3, "token": TOKEN, "method": "tools.call", "params": {"name": "anicraft_run", "arguments": args}}).to_string());
    assert_eq!(live["ok"], true, "{live}");
    let mut headless = Session::default();
    assert_eq!(live["result"], mcp::call_tool(&mut headless, "anicraft_run", &args).unwrap(), "the same dispatcher, the same answer");
    // A protocol error in headless mode is an error reply here; the connection stays usable.
    let bad = send(&mut c, &json!({"id": 4, "token": TOKEN, "method": "tools.call", "params": {"name": "nope"}}).to_string());
    assert_eq!((bad["ok"].clone(), bad["error"].clone()), (json!(false), json!("unknown tool 'nope'")));
    let unknown = send(&mut c, &json!({"id": 5, "token": TOKEN, "method": "frobnicate"}).to_string());
    assert_eq!(unknown["ok"], false);
    let doc = send(&mut c, &json!({"id": 6, "token": TOKEN, "method": "tools.call", "params": {"name": "anicraft_document"}}).to_string());
    // `state` numbers saved states across every session in the process, so it differs.
    let described = |r: &Value| {
        let mut v: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("state");
        v
    };
    assert_eq!(described(&doc["result"]), described(&mcp::call_tool(&mut headless, "anicraft_document", &json!({})).unwrap()));
    let s = app.session();
    assert_eq!(s.visible_elements().len(), 1);
    assert!(s.doc == headless.doc);
}

#[test]
fn a_call_that_waited_past_its_deadline_does_not_run() {
    let (reply, answer) = channel();
    let r = Request { name: "anicraft_run".into(), arguments: json!({"command": "draw.rect", "params": {"x": 0, "y": 0, "w": 5, "h": 5}}), deadline: Instant::now(), reply };
    assert!(r.expired() && r.changes_document());
    let mut s = Session::default();
    r.run(&mut s);
    assert_eq!(answer.recv().unwrap(), Err(BUSY.to_string()));
    assert!(s.visible_elements().is_empty());
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn the_bridge_starts_without_the_app_and_says_how_to_start_it() {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, free_port()));
    let dir = std::env::temp_dir().join(format!("anicraft-bridge-none-{}", std::process::id()));
    let mut b = Bridge::with_token_file(addr, Some(dir.join(CONTROL_FILE)));
    // The handshake and the tool list need no app.
    let init = mcp::handle(&mut b, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
    assert!(init["result"]["instructions"].as_str().unwrap().contains("open"));
    let list = mcp::handle(&mut b, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
    assert_eq!(list["result"]["tools"], mcp::tools());
    let r = mcp::handle(&mut b, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"anicraft_document"}}"#).unwrap();
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("anicraft --control"), "{r}");
    // A token file but no app listening: the same advice.
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(CONTROL_FILE), ControlFile { port: addr.port(), token: TOKEN.into(), pid: 1, version: String::new() }.to_json().to_string()).unwrap();
    assert!(matches!(b.call_tool("anicraft_document", &json!({})), Err(BridgeError::NotRunning(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_bridge_drives_the_app_and_says_when_it_has_gone() {
    let dir = std::env::temp_dir().join(format!("anicraft-bridge-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(CONTROL_FILE);
    let app = App::start(Session::default());
    std::fs::write(&file, ControlFile { port: app.port, token: TOKEN.into(), pid: 1, version: String::new() }.to_json().to_string()).unwrap();
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, app.port));
    let mut b = Bridge::with_token_file(addr, Some(file.clone()));
    let r = b.call("anicraft_run", &json!({"command": "draw.rect", "params": {"x": 0, "y": 0, "w": 9, "h": 9}})).unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(b.call("anicraft_document", &json!({})).unwrap()["isError"], false);
    assert_eq!(app.session().visible_elements().len(), 1, "the edit landed in the app's session");
    // The app's frame loop is gone: the call fails (as an error the assistant reads), never
    // silently.
    let r = b.call("anicraft_run", &json!({"command": "draw.rect", "params": {"x": 0, "y": 0, "w": 9, "h": 9}}));
    assert!(r.as_ref().map_or(true, |v| v["isError"] == true), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
