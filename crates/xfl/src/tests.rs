//! Import tests on a small, self-authored XFL package.

use crate::import::{XflError, import_with};
use anicraft_doc::{ElementKind, LayerKind, LoopMode, Paint, Rgba, SymbolKind, evaluate};
use std::collections::HashMap;

const DOC: &str = r##"<DOMDocument xmlns="http://ns.adobe.com/xfl/2008/" width="640" height="360" frameRate="30" backgroundColor="#336699">
  <symbols><Include href="Characters/Ball.xml"/><Include href="Missing.xml"/></symbols>
  <timelines>
    <DOMTimeline name="Scene 1">
      <layers>
        <DOMLayer name="Ball" color="#FF4FFF">
          <frames>
            <DOMFrame index="0" duration="10" tweenType="motion" acceleration="-50" name="start">
              <elements>
                <DOMSymbolInstance libraryItemName="Characters/Ball" loop="play once" firstFrame="2" blendMode="hardlight">
                  <matrix><Matrix a="2" d="2" tx="100" ty="50"/></matrix>
                  <color><Color alphaMultiplier="0.5"/></color>
                  <filters><DropShadowFilter distance="8" color="#333333" alpha="0.5"/><BlurFilter blurX="10" blurY="10" quality="2"/><BevelFilter/><GlowFilter enabled="false"/></filters>
                </DOMSymbolInstance>
              </elements>
            </DOMFrame>
            <DOMFrame index="10" duration="5">
              <Actionscript><script><![CDATA[stop();]]></script></Actionscript>
              <elements>
                <DOMSymbolInstance libraryItemName="Characters/Ball"><matrix><Matrix a="2" d="2" tx="300" ty="50"/></matrix></DOMSymbolInstance>
              </elements>
            </DOMFrame>
          </frames>
        </DOMLayer>
        <DOMLayer name="Ground" locked="true">
          <frames>
            <DOMFrame index="0" duration="15">
              <elements>
                <DOMShape>
                  <fills><FillStyle index="1"><SolidColor color="#00AA00"/></FillStyle>
                         <FillStyle index="2"><LinearGradient><matrix><Matrix a="0.1" d="0.1" tx="320"/></matrix>
                           <GradientEntry color="#000000" ratio="0"/><GradientEntry color="#FFFFFF" ratio="1"/></LinearGradient></FillStyle></fills>
                  <strokes><StrokeStyle index="1"><SolidStroke weight="3"><fill><SolidColor color="#112233"/></fill></SolidStroke></StrokeStyle></strokes>
                  <edges>
                    <Edge fillStyle1="1" strokeStyle="1" edges="!0 6000|12800 6000|12800 7200"/>
                    <Edge fillStyle0="1" edges="!0 6000|0 7200|12800 7200"/>
                  </edges>
                </DOMShape>
                <DOMStaticText/>
                <DOMBitmapInstance libraryItemName="photo.png"/>
              </elements>
            </DOMFrame>
          </frames>
        </DOMLayer>
        <DOMLayer name="Notes" layerType="guide"><frames><DOMFrame index="0"><elements/></DOMFrame></frames></DOMLayer>
        <DOMLayer name="Folder" layerType="folder"/>
      </layers>
    </DOMTimeline>
    <DOMTimeline name="Scene 2"/>
  </timelines>
</DOMDocument>"##;

const BALL: &str = r##"<DOMSymbolItem name="Characters/Ball" symbolType="movie clip">
  <timeline><DOMTimeline name="Ball"><layers><DOMLayer name="Layer 1"><frames><DOMFrame index="0" duration="4"><Actionscript><script><![CDATA[gotoAndPlay(2);]]></script></Actionscript><elements>
    <DOMOvalObject x="-10" y="-10" objectWidth="20" objectHeight="20"><fill><SolidColor color="#FF0000"/></fill></DOMOvalObject>
  </elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timeline>
</DOMSymbolItem>"##;

fn package() -> HashMap<&'static str, &'static str> {
    HashMap::from([("DOMDocument.xml", DOC), ("LIBRARY/Characters/Ball.xml", BALL)])
}

fn import(files: &HashMap<&'static str, &'static str>) -> Result<(anicraft_doc::Document, crate::Report), XflError> {
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(files.get(p).map(|s| s.as_bytes().to_vec())) };
    import_with(&read)
}

#[test]
fn stage_layers_frames_and_tweens() {
    let (doc, report) = import(&package()).unwrap();
    assert_eq!((doc.stage.width, doc.stage.height, doc.stage.fps), (640, 360, 30.0));
    assert_eq!(doc.stage.background, Rgba::hex(0x336699));
    let names: Vec<&str> = doc.timeline(0).layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Ball", "Ground", "Notes", "Folder"], "folders come across as folders, order kept (top first)");
    assert_eq!(doc.timeline(0).layers[3].kind, LayerKind::Folder);
    assert!(doc.timeline(0).layers[3].keyframes.is_empty(), "a folder has no frames");
    let ball = &doc.timeline(0).layers[0];
    assert_eq!(ball.keyframes.len(), 2);
    assert_eq!((ball.keyframes[0].start, ball.keyframes[0].duration), (0, 10));
    assert_eq!(ball.keyframes[0].label, "start");
    assert!(matches!(ball.keyframes[0].tween.map(|t| t.easing), Some(anicraft_doc::Easing::Classic(e)) if e == -50.0));
    assert!(doc.timeline(0).layers[1].locked);
    assert_eq!(doc.timeline(0).layers[2].kind, LayerKind::Guide);
    assert_eq!(doc.timeline(0).frame_count(), 15);
    assert_eq!(doc.frame_count(), 16, "the movie: scene 1, then the one-frame scene 2");
    // What couldn't come across is reported, not guessed.
    assert!(report.skipped.keys().all(|k| !k.contains("text")), "an empty text field makes nothing and reports nothing");
    assert_eq!(report.skipped.get("instances of missing bitmaps"), Some(&1));
    assert_eq!(report.skipped.get("missing library files"), Some(&1));
    // Frame scripts, on the scene's frames and inside symbols, are counted (ADR 0013).
    assert_eq!(report.skipped.get("frame scripts (ActionScript isn't supported yet)"), Some(&2));
    // Every scene comes across, in order.
    assert_eq!(doc.scenes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Scene 1", "Scene 2"]);
    assert_eq!(doc.timeline(1).layers.len(), 1, "an empty scene gets a layer to draw on");
}

#[test]
fn mask_layers_mask_the_layers_that_point_at_them() {
    // A mask inside a folder: `parentLayerIndex` counts every layer in the file, folders too.
    let shape = r##"<DOMShape><fills><FillStyle index="1"><SolidColor color="#FF0000"/></FillStyle></fills>
      <edges><Edge fillStyle1="1" edges="!0 0|2000 0|2000 2000|0 2000|0 0"/></edges></DOMShape>"##;
    let layer = |attrs: &str| format!(r#"<DOMLayer {attrs}><frames><DOMFrame index="0"><elements>{shape}</elements></DOMFrame></frames></DOMLayer>"#);
    let doc = format!(
        r#"<DOMDocument width="100" height="100"><timelines><DOMTimeline name="Scene 1"><layers>
        <DOMLayer name="Group" layerType="folder"/>{}{}{}{}</layers></DOMTimeline></timelines></DOMDocument>"#,
        layer(r#"name="Spot" layerType="mask" parentLayerIndex="0""#),
        layer(r#"name="Ball" layerType="masked" parentLayerIndex="1""#),
        layer(r#"name="Ring" parentLayerIndex="1""#),
        layer(r#"name="Sky" parentLayerIndex="0""#),
    );
    let doc: &'static str = Box::leak(doc.into_boxed_str());
    let (d, report) = import(&HashMap::from([("DOMDocument.xml", doc)])).unwrap();
    let l = &d.timeline(0).layers;
    assert_eq!(l.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Group", "Spot", "Ball", "Ring", "Sky"]);
    assert_eq!((l[0].kind, l[1].kind), (LayerKind::Folder, LayerKind::Mask));
    assert_eq!(l[1].parent, Some(l[0].id), "the mask is in the folder");
    assert_eq!((l[2].parent, l[3].parent), (Some(l[1].id), Some(l[1].id)), "masked");
    assert_eq!(l[4].parent, Some(l[0].id), "in the folder, not masked");
    assert!(report.skipped.keys().all(|k| !k.contains("mask")));
    let dl = evaluate(&d, 0);
    assert_eq!(dl.clip_parents.len(), 1);
    assert_eq!(dl.items.iter().filter(|i| i.clip == Some(0)).count(), 2);
    assert_eq!(dl.items.iter().filter(|i| i.masks == Some(0)).count(), 1);
}

#[test]
fn symbols_instances_and_the_tween_between_them() {
    let (doc, _) = import(&package()).unwrap();
    assert_eq!(doc.library.len(), 1);
    let sym = &doc.library[0];
    assert_eq!((sym.name.as_str(), sym.kind), ("Ball", SymbolKind::MovieClip));
    let inst = &doc.timeline(0).layers[0].keyframes[0].elements[0];
    let ElementKind::Symbol { symbol, loop_mode, first_frame, blend, filters } = &inst.kind else { panic!("an instance") };
    assert_eq!(*blend, anicraft_doc::BlendMode::HardLight, "Animate's blend mode names");
    assert_eq!(filters.len(), 2, "{filters:?}");
    assert!(matches!(filters[0], anicraft_doc::Filter::DropShadow { distance, inner: false, .. } if distance == 8.0));
    assert!(matches!(filters[1], anicraft_doc::Filter::Blur { blur_x, quality: 2, .. } if blur_x == 10.0));
    assert_eq!((*symbol, *loop_mode, *first_frame), (sym.id, LoopMode::PlayOnce, 2));
    assert_eq!(inst.transform.m, [2.0, 0.0, 0.0, 2.0, 100.0, 50.0]);
    assert_eq!(inst.color.alpha, 0.5);
    // Half-way through the tween (eased), the ball is between its two positions.
    let dl = evaluate(&doc, 5);
    let ball = dl.items.iter().find(|i| matches!(i.fill, Some(Paint::Solid(c)) if c == Rgba::hex(0xff0000))).expect("the ball renders");
    let x = ball.transform.translation().x;
    assert!(x > 100.0 && x < 300.0, "{x}");
}

#[test]
fn shapes_become_fills_and_strokes() {
    let (doc, _) = import(&package()).unwrap();
    let els = &doc.timeline(0).layers[1].keyframes[0].elements;
    assert_eq!(els.len(), 2, "one fill (style 2 has no edges), one stroke");
    let ElementKind::Shape(fill) = &els[0].kind else { panic!() };
    assert_eq!(fill.fill, Some(Paint::Solid(Rgba::hex(0x00aa00))));
    let b = fill.path.bounds();
    assert!((b.min.y - 300.0).abs() < 0.01 && (b.max.x - 640.0).abs() < 0.01 && (b.max.y - 360.0).abs() < 0.01, "{b:?}: twips to pixels");
    let ElementKind::Shape(st) = &els[1].kind else { panic!() };
    assert_eq!(st.stroke.map(|s| (s.color, s.width)), Some((Rgba::hex(0x112233), 3.0)));
    assert!(st.fill.is_none());
}

#[test]
fn a_fla_archive_imports_the_same() {
    let z = crate::zip::tests::make_zip(&[("DOMDocument.xml", DOC.as_bytes(), true), ("LIBRARY/Characters/Ball.xml", BALL.as_bytes(), true)]);
    let dir = std::env::temp_dir().join(format!("anicraft-xfl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fla = dir.join("scene.fla");
    std::fs::write(&fla, &z).unwrap();
    let (doc, _) = crate::import_path(&fla).unwrap();
    assert_eq!(doc.timeline(0).layers.len(), 4);
    assert_eq!(doc.library.len(), 1);
    std::fs::remove_file(&fla).ok();
    std::fs::remove_dir(&dir).ok();
}

#[test]
fn hostile_packages_fail_cleanly() {
    let mut files = package();
    files.insert("DOMDocument.xml", "<DOMDocument><timelines><DOMTimeline><layers><DOMLayer><frames><DOMFrame index=\"99999999999\" duration=\"-4\"><elements><DOMShape><edges><Edge fillStyle1=\"1\" edges=\"!#FFFFFFFF.FF 1e30|nan 0[\"/></edges></DOMShape></elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>");
    let (doc, _) = import(&files).unwrap();
    assert!(doc.frame_count() <= 100_001);
    files.insert("DOMDocument.xml", "<NotAnimate/>");
    assert!(matches!(import(&files), Err(XflError::NotXfl(_))));
    files.insert("DOMDocument.xml", "<DOMDocument><unclosed>");
    assert!(matches!(import(&files), Err(XflError::Xml(_))));
    assert!(import(&HashMap::new()).is_err(), "no DOMDocument.xml");
}

#[test]
fn shape_tweens_come_across_as_shape_tweens() {
    let shape = |edges: &str| {
        format!(r##"<DOMShape><fills><FillStyle index="1"><SolidColor color="#3366FF"/></FillStyle></fills><edges><Edge fillStyle1="1" edges="{edges}"/></edges></DOMShape>"##)
    };
    let doc = format!(
        r#"<DOMDocument width="400" height="200"><timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="Morph"><frames>
          <DOMFrame index="0" duration="10" tweenType="shape" acceleration="40"><elements>{}</elements></DOMFrame>
          <DOMFrame index="10"><elements>{}</elements></DOMFrame>
        </frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"#,
        shape("!0 0|2000 0|2000 2000|0 2000|0 0"),
        shape("!4000 0|6000 1000|4000 2000|4000 0"),
    );
    let doc: &'static str = Box::leak(doc.into_boxed_str());
    let (d, report) = import(&HashMap::from([("DOMDocument.xml", doc)])).unwrap();
    let k = &d.timeline(0).layers[0].keyframes[0];
    let t = k.tween.expect("a tween");
    assert_eq!(t.kind, anicraft_doc::TweenKind::Shape);
    assert!(matches!(t.easing, anicraft_doc::Easing::Classic(a) if a == 40.0));
    assert!(report.skipped.keys().all(|s| !s.contains("shape tween")), "{:?}", report.skipped);
    // Halfway, the square has morphed towards the triangle.
    let mid = evaluate(&d, 5).items[0].path.bounds();
    assert!(mid.max.x > 110.0 && mid.min.x > 10.0, "{mid:?}");
}

fn png_2x1() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 2, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    }
    out
}

const BITMAP_DOC: &str = r##"<DOMDocument width="200" height="100">
  <media>
    <DOMBitmapItem name="Art/photo.png" href="Art/photo.png" bitmapDataHRef="M 1 1.dat"/>
    <DOMBitmapItem name="packed.png" href="packed.png" bitmapDataHRef="M 2 2.dat"/>
    <DOMBitmapItem name="private.jpg" href="private.jpg" bitmapDataHRef="M 3 3.dat"/>
    <DOMBitmapItem name="gone.png" href="gone.png"/>
  </media>
  <timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="Layer 1"><frames><DOMFrame index="0"><elements>
    <DOMBitmapInstance libraryItemName="Art/photo.png"><matrix><Matrix a="10" d="10" tx="5" ty="6"/></matrix></DOMBitmapInstance>
    <DOMBitmapInstance libraryItemName="packed.png"/>
    <DOMBitmapInstance libraryItemName="gone.png"/>
    <DOMShape>
      <fills><FillStyle index="1"><BitmapFill bitmapPath="Art/photo.png"><matrix><Matrix a="40" d="40" tx="100" ty="50"/></matrix></BitmapFill></FillStyle>
             <FillStyle index="2"><BitmapFill bitmapPath="private.jpg" bitmapIsClipped="true"/></FillStyle></fills>
      <edges><Edge fillStyle1="1" edges="!2000 1000|4000 1000|4000 2000|2000 2000|2000 1000"/>
             <Edge fillStyle1="2" edges="!0 0|200 0|200 200|0 0"/></edges>
    </DOMShape>
  </elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines>
</DOMDocument>"##;

#[test]
fn bitmaps_come_across_when_their_image_is_png_or_jpeg() {
    let files: HashMap<String, Vec<u8>> = HashMap::from([
        ("DOMDocument.xml".to_string(), BITMAP_DOC.as_bytes().to_vec()),
        // An XFL folder keeps the original image in LIBRARY/.
        ("LIBRARY/Art/photo.png".to_string(), png_2x1()),
        // A .fla may hold only the data file; a PNG or JPEG there is used as it is.
        ("bin/M 2 2.dat".to_string(), png_2x1()),
        // Animate's own encoding is not public: reported, not guessed.
        ("bin/M 3 3.dat".to_string(), vec![3, 5, 0, 0, 1, 2, 3]),
    ]);
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(files.get(p).cloned()) };
    let (doc, report) = import_with(&read).unwrap();
    assert_eq!(doc.bitmaps.iter().map(|b| (b.name.as_str(), b.width, b.height)).collect::<Vec<_>>(), [("photo.png", 2, 1), ("packed.png", 2, 1)]);
    assert_eq!(report.skipped.get("bitmaps stored in Animate's own format"), Some(&1));
    assert_eq!(report.skipped.get("missing bitmap files"), Some(&1));
    assert_eq!(report.skipped.get("instances of missing bitmaps"), Some(&1));
    assert_eq!(report.skipped.get("bitmap fills of missing bitmaps"), Some(&1));

    let els = &doc.timeline(0).layers[0].keyframes[0].elements;
    assert_eq!(els.len(), 3, "two bitmap instances and the bitmap-filled shape");
    let photo = doc.bitmaps[0].id;
    // An instance is a rectangle of the image's size, placed by its matrix.
    let ElementKind::Shape(sh) = &els[0].kind else { panic!() };
    assert_eq!(sh.fill, Some(Paint::Bitmap { bitmap: photo, repeat: false, matrix: anicraft_geom::Affine::IDENTITY }));
    assert_eq!(sh.path.bounds(), anicraft_geom::Rect::new(0.0, 0.0, 2.0, 1.0));
    assert_eq!(els[0].transform.m, [10.0, 0.0, 0.0, 10.0, 5.0, 6.0]);
    // A bitmap fill keeps its matrix, in pixels (twips scale removed), and repeats.
    let ElementKind::Shape(fill) = &els[2].kind else { panic!() };
    assert_eq!(fill.fill, Some(Paint::Bitmap { bitmap: photo, repeat: true, matrix: anicraft_geom::Affine { m: [2.0, 0.0, 0.0, 2.0, 100.0, 50.0] } }));
    // The renderer gets the pixels with the items.
    let list = evaluate(&doc, 0);
    assert_eq!(list.items.iter().filter(|i| i.image.is_some()).count(), 3);
}
