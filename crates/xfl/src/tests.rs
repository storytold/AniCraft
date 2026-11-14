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
                  <filters><DropShadowFilter distance="8" color="#333333" alpha="0.5"/><BlurFilter blurX="10" blurY="10" quality="2"/><GlowFilter enabled="false"/>
                    <BevelFilter blurX="4" blurY="6" distance="3" angle="135" highlightColor="#FFFF00" highlightAlpha="0.75" shadowAlpha="0.5" type="full" knockout="true"/>
                    <GradientGlowFilter strength="2" type="inner"><GradientEntry color="#FFFFFF" alpha="0.5" ratio="0"/><GradientEntry color="#FF0000" ratio="1"/><GradientEntry color="#00FF00" ratio="128"/><GradientEntry color="#0000FF" ratio="255"/></GradientGlowFilter>
                    <GradientBevelFilter quality="3"><GradientEntry color="#FFFFFF" ratio="0"/><GradientEntry color="#000000" ratio="1"/></GradientBevelFilter>
                    <SparkleFilter/></filters>
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
        <DOMLayer name="Ground" locked="true" heightMultiplier="2">
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
    assert!(matches!(ball.keyframes[0].tween.as_ref().map(|t| t.easing.clone()), Some(anicraft_doc::Easing::Classic(e)) if e == -50.0));
    assert!(doc.timeline(0).layers[1].locked);
    assert_eq!((doc.timeline(0).layers[0].height(), doc.timeline(0).layers[1].height()), (1, 2), "Layer Properties height");
    assert_eq!(doc.timeline(0).layers[2].kind, LayerKind::Guide);
    assert_eq!(doc.timeline(0).frame_count(), 15);
    assert_eq!(doc.frame_count(), 16, "the movie: scene 1, then the one-frame scene 2");
    // What couldn't come across is reported, not guessed.
    assert!(report.skipped.keys().all(|k| !k.contains("text")), "an empty text field makes nothing and reports nothing");
    assert_eq!(report.skipped.get("instances of missing bitmaps"), Some(&1));
    assert_eq!(report.skipped.get("missing library files"), Some(&1));
    assert_eq!(report.skipped.get("unknown filters"), Some(&1));
    // Frame scripts, on the scene's frames and inside symbols, are kept verbatim (ADR 0013
    // tier 1); these are in the subset that runs (tier 2).
    assert_eq!(report.runs.get("frame scripts in ActionScript 3.0"), Some(&2));
    // The symbol's path puts it in a Library folder (ADR 0050).
    assert_eq!(report.kept.keys().collect::<Vec<_>>(), ["library folders"], "{report:?}");
    assert_eq!(doc.entry_path(anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Symbol(doc.library[0].id))), "Characters/Ball");
    assert!(report.skipped.keys().all(|k| !k.contains("script")), "{report:?}");
    assert_eq!(ball.keyframes[1].script.as_ref().and_then(|s| s.source()).map(|s| s.text.as_str()), Some("stop();"));
    let inner = &doc.library[0].timeline.layers[0].keyframes[0];
    assert!(matches!(inner.script.as_ref().and_then(|s| s.source()), Some(s) if s.text == "gotoAndPlay(2);" && s.language == anicraft_doc::Language::ActionScript3));
    assert!(report.summary().contains("Runs in Test Movie: 2 frame scripts in ActionScript 3.0."), "{}", report.summary());
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
    let ElementKind::Symbol { symbol, loop_mode, first_frame, blend, filters: entries, .. } = &inst.kind else { panic!("an instance") };
    assert_eq!(*blend, anicraft_doc::BlendMode::HardLight, "Animate's blend mode names");
    // In order; the switched-off glow comes in switched off, keeping its place (ADR 0091).
    assert_eq!(entries.iter().map(|e| e.enabled).collect::<Vec<_>>(), [true, true, false, true, true, true]);
    assert!(matches!(entries[2].filter, anicraft_doc::Filter::Glow { .. }));
    let filters: Vec<anicraft_doc::Filter> = entries.iter().filter(|e| e.enabled).map(|e| e.filter.clone()).collect();
    assert_eq!(filters.len(), 5, "{filters:?}");
    assert!(matches!(filters[0], anicraft_doc::Filter::DropShadow { distance, inner: false, .. } if distance == 8.0));
    assert!(matches!(filters[1], anicraft_doc::Filter::Blur { blur_x, quality: 2, .. } if blur_x == 10.0));
    use anicraft_doc::{Filter, FilterSide};
    let Filter::Bevel { blur_x, blur_y, distance, angle, highlight_color, shadow_color, side, knockout, strength, .. } = filters[2] else { panic!("{:?}", filters[2]) };
    assert_eq!((blur_x, blur_y, distance, angle, side, knockout, strength), (4.0, 6.0, 3.0, 135.0, FilterSide::Full, true, 1.0));
    assert_eq!((highlight_color, shadow_color), (Rgba::hex(0xffff00).with_alpha(0.75), Rgba::BLACK.with_alpha(0.5)));
    // Ramps: ratios 0..1 or 0..255; the glow's start pinned transparent, the bevel given a
    // transparent middle (as Animate's panel keeps them).
    let Filter::GradientGlow { stops, side: FilterSide::Inner, strength, .. } = &filters[3] else { panic!("{:?}", filters[3]) };
    assert_eq!(*strength, 2.0);
    assert_eq!(stops.iter().map(|s| (s.offset, s.color.a)).collect::<Vec<_>>(), vec![(0.0, 0.0), (1.0 / 255.0, 1.0), (128.0 / 255.0, 1.0), (1.0, 1.0)], "one byte scale for every ratio");
    let Filter::GradientBevel { stops, side: FilterSide::Inner, quality: 3, .. } = &filters[4] else { panic!("{:?}", filters[4]) };
    assert_eq!(stops.len(), 3);
    assert_eq!((stops[1].offset, stops[1].color.a), (0.5, 0.0));
    assert_eq!((*symbol, *loop_mode, *first_frame), (sym.id, LoopMode::PlayOnce, 2));
    assert_eq!(inst.transform.m, [2.0, 0.0, 0.0, 2.0, 100.0, 50.0]);
    assert_eq!(inst.color, anicraft_doc::ColorEffect::Alpha(0.5));
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
    assert_eq!(st.stroke.as_ref().map(|s| (s.color, s.width)), Some((Rgba::hex(0x112233), 3.0)));
    assert!(st.fill.is_none());
}

#[test]
fn a_fla_archive_imports_the_same() {
    let z = crate::zip::pack::write(&[crate::zip::pack::Raw::new("DOMDocument.xml", DOC.as_bytes(), true), crate::zip::pack::Raw::new("LIBRARY/Characters/Ball.xml", BALL.as_bytes(), true)], false);
    let dir = std::env::temp_dir().join(format!("anicraft-xfl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fla = dir.join("scene.fla");
    std::fs::write(&fla, &z).unwrap();
    let (doc, _) = crate::import_path(&fla).unwrap();
    assert_eq!(doc.timeline(0).layers.len(), 4);
    assert_eq!(doc.library.len(), 1);
    let (from_bytes, _) = crate::import_fla(&z).unwrap();
    assert_eq!((from_bytes.timeline(0).layers.len(), from_bytes.library.len()), (4, 1), "the bytes a browser hands over import the same");
    assert!(crate::import_fla(b"not a zip").is_err());
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
    let t = k.tween.clone().expect("a tween");
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
  <folders><DOMFolderItem name="Art" isExpanded="true"/><DOMFolderItem name="Empty/Inner"/></folders>
  <media>
    <DOMBitmapItem name="Art/photo.png" href="Art/photo.png" bitmapDataHRef="M 1 1.dat"/>
    <DOMBitmapItem name="packed.png" href="packed.png" bitmapDataHRef="M 2 2.dat" allowSmoothing="false"/>
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
    // Library folders come across (ADR 0050): the item's path places it, empty ones are kept,
    // and Allow Smoothing is read (on unless the file turns it off).
    let path = |i| doc.entry_path(anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Bitmap(i)));
    assert_eq!((path(doc.bitmaps[0].id).as_str(), path(doc.bitmaps[1].id).as_str()), ("Art/photo.png", "packed.png"));
    let folders: Vec<String> = doc.library_tree.folders.iter().map(|f| doc.entry_path(anicraft_doc::LibraryEntry::Folder(f.id))).collect();
    assert_eq!(folders, ["Art", "Empty", "Empty/Inner"]);
    assert_eq!(report.kept.get("library folders"), Some(&1));
    assert_eq!((doc.bitmaps[0].smoothing, doc.bitmaps[1].smoothing), (true, false));
    assert!(list.items.iter().any(|i| i.image.is_some() && !i.smooth));
}

/// A square (0, 0)–(100, 100) shape-tweening to a triangle (200, 0), (300, 50), (200, 100),
/// with `first` and `second` inside the two frames.
fn hinted_doc(first: &str, second: &str) -> &'static str {
    let shape = |edges: &str| {
        format!(r##"<DOMShape><fills><FillStyle index="1"><SolidColor color="#3366FF"/></FillStyle></fills><edges><Edge fillStyle1="1" edges="{edges}"/></edges></DOMShape>"##)
    };
    let doc = format!(
        r#"<DOMDocument width="400" height="200"><timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="Morph"><frames>
          <DOMFrame index="0" duration="10" tweenType="shape">{first}<elements>{}</elements></DOMFrame>
          <DOMFrame index="10">{second}<elements>{}</elements></DOMFrame>
        </frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"#,
        shape("!0 0|2000 0|2000 2000|0 2000|0 0"),
        shape("!4000 0|6000 1000|4000 2000|4000 0"),
    );
    Box::leak(doc.into_boxed_str())
}

#[test]
fn shape_hints_come_across_in_either_form() {
    use anicraft_geom::{PathCmd, Vec2};
    let both_in_one = hinted_doc(r#"<shapeHints><ShapeHint index="0" startPoint="0, 0" endPoint="300, 50"/></shapeHints>"#, "");
    // One list per keyframe, matched by letter; the second point a pixel off the edge snaps on.
    let per_key = hinted_doc(r#"<shapeHints><ShapeHint name="a" x="0" y="0"/></shapeHints>"#, r#"<shapeHints><ShapeHint name="a" x="299" y="50.5"/></shapeHints>"#);
    for doc in [both_in_one, per_key] {
        let (d, report) = import(&HashMap::from([("DOMDocument.xml", doc)])).unwrap();
        assert!(report.skipped.keys().all(|s| !s.contains("hint")), "{:?}", report.skipped);
        let l = &d.timeline(0).layers[0];
        let k = &l.keyframes[0];
        assert_eq!(k.hints.len(), 1);
        assert!(anicraft_doc::placed_on(&k.elements, k.hints[0].start).is_some());
        assert!(anicraft_doc::placed_on(&l.keyframes[1].elements, k.hints[0].end).is_some(), "{:?}", k.hints[0].end);
        // Halfway the square's corner is halfway to the triangle's tip.
        let dl = evaluate(&d, 5);
        let want = Vec2::new(0.0, 0.0).lerp(k.hints[0].end, 0.5);
        let it = &dl.items[0];
        let hit = it.path.cmds.iter().any(|c| matches!(*c, PathCmd::MoveTo(a) | PathCmd::CubicTo(_, _, a) if it.transform.apply(a).distance(want) < 0.05));
        assert!(hit, "{want:?} {:?}", it.path.cmds);
    }
}

#[test]
fn odd_shape_hints_are_reported_not_fatal() {
    let many: String = (0..30).map(|i| format!(r#"<ShapeHint index="{i}" startPoint="0, 0" endPoint="200, 0"/>"#)).collect();
    let doc = hinted_doc(&format!("<shapeHints>{many}<ShapeHint/><ShapeHint x=\"nan\" y=\"1\"/></shapeHints>"), "");
    let (d, report) = import(&HashMap::from([("DOMDocument.xml", doc)])).unwrap();
    assert_eq!(d.timeline(0).layers[0].keyframes[0].hints.len(), 26);
    assert!(report.skipped.contains_key("shape hints past z"), "{:?}", report.skipped);
    assert!(report.skipped.contains_key("shape hints without a position"));
    // Only start points and nothing on the end keyframe.
    let doc = hinted_doc(r#"<shapeHints><ShapeHint x="0" y="0"/></shapeHints>"#, "");
    let (d, report) = import(&HashMap::from([("DOMDocument.xml", doc)])).unwrap();
    assert!(d.timeline(0).layers[0].keyframes[0].hints.is_empty());
    assert!(report.skipped.contains_key("shape hints missing on the end keyframe"));
}

#[test]
fn eases_names_and_loop_modes_the_corpus_found_missing() {
    let frame = |attrs: &str, inner: &str| format!(r#"<DOMFrame index="0" duration="4" tweenType="motion" {attrs}>{inner}<elements><DOMSymbolInstance libraryItemName="Clip" name="my clip" loop="loop reverse"/></elements></DOMFrame><DOMFrame index="4"><elements/></DOMFrame>"#);
    let pts = |p: &[(f32, f32)]| p.iter().map(|(x, y)| format!(r#"<Point x="{x}" y="{y}"/>"#)).collect::<String>();
    let layers = [
        // Seven points: a curve of two segments, exactly.
        frame(r#"hasCustomEase="true" acceleration="30""#, &format!(r#"<tweens><CustomEase target="all">{}</CustomEase></tweens>"#, pts(&[(0.0, 0.0), (0.2, 0.0), (0.3, 0.5), (0.5, 0.5), (0.7, 0.5), (0.8, 1.0), (1.0, 1.0)]))),
        // One curve per property: each its own; the slider eases the rest.
        frame(r#"hasCustomEase="true" useSingleEaseCurve="false" acceleration="-50""#, &format!(r#"<tweens><CustomEase target="position">{0}</CustomEase><CustomEase target="rotation">{1}</CustomEase><CustomEase target="sparkle">{0}</CustomEase></tweens>"#, pts(&[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)]), pts(&[(0.0, 0.0), (0.1, 0.4), (0.2, 1.2), (0.4, 1.2), (0.6, 1.2), (0.9, 1.0), (1.0, 1.0)]))),
        // Said to have a custom ease, with none there; and a broken point list.
        frame(r#"hasCustomEase="true""#, ""),
        frame(r#"hasCustomEase="true" acceleration="10""#, &format!(r#"<tweens><CustomEase target="all">{}</CustomEase></tweens>"#, pts(&[(0.0, 0.0), (0.5, 0.0), (1.0, 1.0)]))),
    ];
    let layers: String = layers.iter().map(|f| format!(r#"<DOMLayer name="L"><frames>{f}</frames></DOMLayer>"#)).collect();
    let doc = format!(r#"<DOMDocument><symbols><Include href="Clip.xml"/></symbols><timelines><DOMTimeline name="Scene 1"><layers>{layers}</layers></DOMTimeline></timelines></DOMDocument>"#);
    let doc: &'static str = Box::leak(doc.into_boxed_str());
    let clip = r#"<DOMSymbolItem name="Clip"><timeline><DOMTimeline name="Clip"/></timeline></DOMSymbolItem>"#;
    let (d, report) = import(&HashMap::from([("DOMDocument.xml", doc), ("LIBRARY/Clip.xml", clip)])).unwrap();
    use anicraft_doc::{CustomEase, EaseGroup, Easing};
    let tween = |l: usize| d.timeline(0).layers[l].keyframes[0].tween.clone().unwrap();
    let v = |p: &[(f32, f32)]| p.iter().map(|(x, y)| anicraft_geom::Vec2::new(*x, *y)).collect::<Vec<_>>();
    let two = CustomEase::from_flat(&v(&[(0.0, 0.0), (0.2, 0.0), (0.3, 0.5), (0.5, 0.5), (0.7, 0.5), (0.8, 1.0), (1.0, 1.0)])).unwrap();
    assert_eq!(tween(0).easing, Easing::Custom(std::sync::Arc::new(two)), "a two-segment curve, exactly");
    assert!((tween(0).easing.apply(0.5) - 0.5).abs() < 1e-5 && tween(0).easing.apply(0.25) < 0.4);
    let per = tween(1);
    assert_eq!(per.easing, Easing::Classic(-50.0));
    assert_eq!(per.ease(EaseGroup::Position), &Easing::CubicBezier(0.5, 0.0, 0.5, 1.0));
    assert!(matches!(per.ease(EaseGroup::Rotation), Easing::Custom(c) if c.points().len() == 3));
    assert_eq!((per.has_own(EaseGroup::Scale), per.groups.len()), (false, 2));
    assert_eq!((tween(2).easing, tween(3).easing), (Easing::Linear, Easing::Classic(10.0)));
    assert_eq!(report.skipped.get("custom eases of unknown properties (left out)"), Some(&1));
    assert_eq!(report.skipped.get("custom eases without their curve (drawn with the simple ease)"), Some(&1));
    assert_eq!(report.skipped.get("custom ease curves with a broken point list (drawn with the simple ease)"), Some(&1));
    assert!(!report.skipped.contains_key("custom ease curves (drawn with the simple ease)"));
    assert_eq!(d.library[0].kind, SymbolKind::MovieClip, "no symbolType: a movie clip");
    let ElementKind::Symbol { loop_mode, .. } = &d.timeline(0).layers[0].keyframes[0].elements[0].kind else { panic!() };
    assert_eq!(*loop_mode, LoopMode::Loop, "loop reverse plays forward");
    assert_eq!(report.skipped.get("reverse loop modes (played forward)"), Some(&4));
    assert_eq!(d.timeline(0).layers[0].keyframes[0].elements[0].name, "", "not an identifier: left out");
    assert_eq!(report.skipped.get("instance names actions can't use (left out)"), Some(&4));
}

#[test]
fn scripts_linkage_and_the_document_class_are_kept_not_run() {
    // ADR 0013 tier 1: frame scripts verbatim, linkage classes and the document class as text.
    let doc = r#"<DOMDocument><symbols><Include href="Hero.xml"/><Include href="Old.xml"/><Include href="Bad.xml"/><Include href="Off.xml"/></symbols>
      <media><DOMBitmapItem name="pic.png" linkageExportForAS="true" linkageClassName="Pic"/></media>
      <timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="as"><frames>
        <DOMFrame index="0"><Actionscript><script><![CDATA[var n:int = 1 < 2 ? 1 : 0; // <tags> & "quotes"
trace(n);]]></script></Actionscript><elements><DOMSymbolInstance libraryItemName="Hero"><Actionscript><script><![CDATA[on (release) { play(); }]]></script></Actionscript></DOMSymbolInstance></elements></DOMFrame>
        <DOMFrame index="1"><Actionscript><script><![CDATA[   ]]></script></Actionscript><elements/></DOMFrame>
      </frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"#;
    let sym = |name: &str, attrs: &str| format!(r#"<DOMSymbolItem name="{name}" {attrs}><timeline><DOMTimeline name="{name}"/></timeline></DOMSymbolItem>"#);
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let settings = r#"<flash_profiles><flash_profile name="Old" current="false"><PublishFlashProperties><ActionScriptVersion>3</ActionScriptVersion><DocumentClass>Nope</DocumentClass></PublishFlashProperties></flash_profile>
      <flash_profile name="Default" current="true"><PublishFlashProperties><ActionScriptVersion>2</ActionScriptVersion><DocumentClass> com.example.Main </DocumentClass></PublishFlashProperties></flash_profile></flash_profiles>"#;
    let files = HashMap::from([
        ("DOMDocument.xml", doc),
        ("PublishSettings.xml", settings),
        ("LIBRARY/Hero.xml", leak(sym("Hero", r#"linkageExportForAS="true" linkageClassName="com.example.Hero" linkageBaseClass="flash.display.MovieClip""#))),
        ("LIBRARY/Old.xml", leak(sym("Old", r#"linkageExportForAS="true" linkageIdentifier="oldClip""#))),
        ("LIBRARY/Bad.xml", leak(sym("Bad", r#"linkageExportForAS="true" linkageClassName="not a class""#))),
        ("LIBRARY/Off.xml", leak(sym("Off", r#"linkageExportForAS="false" linkageClassName="Off""#))),
    ]);
    let (d, report) = import(&files).unwrap();
    let s = d.timeline(0).layers[0].keyframes[0].script.as_ref().and_then(|s| s.source()).unwrap();
    assert_eq!(s.text, "var n:int = 1 < 2 ? 1 : 0; // <tags> & \"quotes\"\ntrace(n);", "verbatim");
    assert_eq!(s.language, anicraft_doc::Language::ActionScript1or2, "the current publish profile says ActionScript 2");
    assert!(d.timeline(0).layers[0].keyframes[1].script.is_none(), "a blank script is nothing");
    assert_eq!(d.document_class, "com.example.Main");
    let linkage: Vec<Option<(&str, &str)>> = d.library.iter().map(|s| s.linkage.as_ref().map(|l| (l.class.as_str(), l.base.as_str()))).collect();
    assert_eq!(linkage, [Some(("com.example.Hero", "flash.display.MovieClip")), Some(("oldClip", "")), None, None]);
    let kept: Vec<(&str, usize)> = report.kept.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    assert_eq!(kept, [("document class", 1), ("frame scripts in ActionScript 1.0 & 2.0", 1), ("linkage classes", 2)]);
    assert_eq!(report.skipped.get("linkage classes that aren't class names (left out)"), Some(&1));
    assert_eq!(report.skipped.get("linkage classes on bitmaps and sounds (left out)"), Some(&1));
    assert_eq!(report.skipped.get("scripts on instances (left out; frame scripts are kept)"), Some(&1));
    // Broken or missing publish settings change nothing.
    let mut files = files;
    files.insert("PublishSettings.xml", "<flash_profiles><unclosed>");
    let (d, report) = import(&files).unwrap();
    assert!(d.document_class.is_empty() && !report.kept.contains_key("document class"));
    // Read as ActionScript 3.0, its variable and `?:` are in the subset now (ADR 0052).
    assert!(report.runs.contains_key("frame scripts in ActionScript 3.0"));
}

#[test]
fn loose_shapes_come_in_raw_and_drawing_objects_and_groups_as_groups() {
    // Animate's raw shapes (merge drawing) are loose `DOMShape`s; a drawing object says so, and
    // a group is a group (ADR 0073) whose shapes merge only with each other.
    let square = |attrs: &str, x: u32| {
        format!(
            r##"<DOMShape {attrs}><fills><FillStyle index="1"><SolidColor color="#FF0000"/></FillStyle></fills><strokes><StrokeStyle index="1"><SolidStroke weight="2"><fill><SolidColor/></fill></SolidStroke></StrokeStyle></strokes>
            <edges><Edge fillStyle1="1" strokeStyle="1" edges="!{x} 0|{} 0|{} 2000|{x} 2000|{x} 0"/></edges></DOMShape>"##,
            x + 2000,
            x + 2000
        )
    };
    let doc = format!(
        r#"<DOMDocument width="400" height="100"><timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="Art"><frames><DOMFrame index="0"><elements>{}{}<DOMGroup><members>{}</members></DOMGroup></elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"#,
        square("", 0),
        square(r#"isDrawingObject="true""#, 4000),
        square("", 8000),
    );
    let doc: &'static str = Box::leak(doc.into_boxed_str());
    let (d, _) = import(&HashMap::from([("DOMDocument.xml", doc)])).unwrap();
    let els = &d.timeline(0).layers[0].keyframes[0].elements;
    let merge: Vec<bool> = els.iter().map(|e| matches!(&e.kind, ElementKind::Shape(s) if s.merge)).collect();
    // Each square is a fill and a stroke; the third is in a group (ADR 0073), raw inside it.
    assert_eq!(merge, [true, true, false, false, false]);
    let group = els.last().unwrap();
    assert!(matches!(group.kind, ElementKind::Group { .. }));
    let inside: Vec<bool> = group.kind.group_members().iter().map(|e| matches!(&e.kind, ElementKind::Shape(s) if s.merge)).collect();
    assert_eq!(inside, [true, true]);
}

#[test]
fn stroke_caps_joints_dashes_and_dots_come_across() {
    use anicraft_doc::{Cap, Dash, Join, StrokeStyle};
    let read = |xml: &str| {
        let el = crate::xml::parse(xml.as_bytes()).unwrap();
        let mut report = crate::Report::default();
        let st = crate::import::stroke(&el, &HashMap::new(), &mut report).unwrap();
        (st, report)
    };
    let (st, report) = read(r##"<SolidStroke weight="4" caps="none" joints="miter" miterLimit="7"><fill><SolidColor color="#112233"/></fill></SolidStroke>"##);
    assert_eq!((st.width, st.color), (4.0, Rgba::hex(0x112233)));
    assert_eq!(st.style, StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 7.0, dash: None });
    assert!(report.skipped.is_empty());
    // Round caps and joints and a miter limit of 3 are the format's defaults.
    assert_eq!(read(r#"<SolidStroke weight="1"/>"#).0.style, StrokeStyle::default());
    assert_eq!(read(r#"<SolidStroke caps="square" joints="bevel" miterLimit="1e9"/>"#).0.style, StrokeStyle { cap: Cap::Square, join: Join::Bevel, miter_limit: 100.0, dash: None });
    let (dashed, _) = read(r#"<DashedStroke weight="2" dash1="8" dash2="2.5" caps="square"/>"#);
    assert_eq!(dashed.style, StrokeStyle { cap: Cap::Square, dash: Dash::dashed(8.0, 2.5), ..StrokeStyle::default() });
    // No gap draws solid.
    assert_eq!(read(r#"<DashedStroke dash1="8" dash2="0"/>"#).0.style.dash, None);
    assert_eq!(read(r#"<DottedStroke weight="3" dotSpace="5"/>"#).0.style.dash, Dash::dotted(5.0));
    for kind in ["RaggedStroke", "StippleStroke", "HatchedStroke"] {
        let (st, report) = read(&format!("<{kind} weight=\"2\"/>"));
        assert_eq!(st.style.dash, None);
        assert_eq!(report.skipped.get("ragged, stipple and hatched strokes (drawn solid)"), Some(&1));
    }
}

/// A button whose Over and Down frames hold sounds (ADR 0057), a sound on the main timeline, and
/// sounds the package doesn't hold in a form anyone can read.
const SOUND_DOC: &str = r##"<DOMDocument width="200" height="100">
  <media>
    <DOMSoundItem name="Sounds/click.wav" href="Sounds/click.wav" soundDataHRef="M 1 1.dat"/>
    <DOMSoundItem name="private.mp3" href="private.mp3" soundDataHRef="M 2 2.dat"/>
    <DOMSoundItem name="gone.wav" href="gone.wav"/>
  </media>
  <symbols><Include href="Push.xml"/></symbols>
  <timelines><DOMTimeline name="Scene 1"><layers>
    <DOMLayer name="Music"><frames><DOMFrame index="0" duration="4" soundName="Sounds/click.wav" soundSync="start" soundLoopMode="repeat" soundLoop="3"/></frames></DOMLayer>
    <DOMLayer name="Buttons"><frames><DOMFrame index="0" duration="4"><elements><DOMSymbolInstance libraryItemName="Push"/></elements></DOMFrame></frames></DOMLayer>
  </layers></DOMTimeline></timelines>
</DOMDocument>"##;

const PUSH: &str = r##"<DOMSymbolItem name="Push" symbolType="button"><timeline><DOMTimeline name="Push"><layers>
  <DOMLayer name="Sounds"><frames>
    <DOMFrame index="0"/>
    <DOMFrame index="1" soundName="Sounds/click.wav"/>
    <DOMFrame index="2" soundName="Sounds/click.wav" soundSync="stream" soundEffect="fade in"/>
    <DOMFrame index="3" soundName="private.mp3"/>
  </frames></DOMLayer>
  <DOMLayer name="Art"><frames><DOMFrame index="0" duration="4"><elements><DOMRectangleObject x="0" y="0" objectWidth="40" objectHeight="20"><fill><SolidColor color="#3366CC"/></fill></DOMRectangleObject></elements></DOMFrame></frames></DOMLayer>
</layers></DOMTimeline></timeline></DOMSymbolItem>"##;

#[test]
fn sounds_in_the_package_come_across_on_frames_and_buttons_states() {
    let wav = anicraft_doc::audio::wav::encode(8000, 1, &[0.25; 800], anicraft_doc::audio::wav::Bits::Int16);
    let files: HashMap<String, Vec<u8>> = HashMap::from([
        ("DOMDocument.xml".to_string(), SOUND_DOC.as_bytes().to_vec()),
        ("LIBRARY/Push.xml".to_string(), PUSH.as_bytes().to_vec()),
        // An XFL folder keeps the original file in LIBRARY/.
        ("LIBRARY/Sounds/click.wav".to_string(), wav),
        // Animate's own encoding is not public: reported, not guessed.
        ("bin/M 2 2.dat".to_string(), vec![1, 2, 3, 4, 5, 6, 7, 8]),
    ]);
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(files.get(p).cloned()) };
    let (doc, report) = import_with(&read).unwrap();
    assert_eq!(doc.sounds.iter().map(|s| (s.name.as_str(), s.rate, s.frames)).collect::<Vec<_>>(), [("click.wav", 8000, 800)]);
    let click = doc.sounds[0].id;
    let path = doc.entry_path(anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Sound(click)));
    assert_eq!(path, "Sounds/click.wav", "in its Library folder");
    // The main timeline's: Start, three times.
    let music = doc.timeline(0).layers[0].keyframes[0].sound.clone().unwrap();
    assert_eq!((music.sound, music.sync, music.repeat, music.looping), (click, anicraft_doc::SoundSync::Start, 3, false));
    // The button's Over and Down frames keep theirs; the Hit frame's sound isn't readable.
    let push = doc.library.iter().find(|s| s.name == "Push").unwrap();
    let on = |f: u32| push.timeline.layers[0].keyframes.iter().find(|k| k.start == f).and_then(|k| k.sound.clone());
    assert_eq!(on(1).map(|s| (s.sound, s.sync)), Some((click, anicraft_doc::SoundSync::Event)));
    assert_eq!(on(2).map(|s| (s.sound, s.sync)), Some((click, anicraft_doc::SoundSync::Stream)));
    assert_eq!(on(3), None);
    assert_eq!(on(2).map(|s| s.effect), Some(anicraft_doc::sound::SoundEffect::FadeIn), "the effect comes across (ADR 0086)");
    // Sounds that don't come across are named, with their format (ADR 0086).
    assert_eq!(report.names.get("sounds in formats AniCraft can't decode"), Some(&vec!["private.mp3: Animate's own sound data".to_string()]));
    assert_eq!(report.names.get("missing sound files"), Some(&vec!["gone.wav".to_string()]));
    assert_eq!(report.skipped.get("frame sounds whose sound isn't in the package"), Some(&1));
    assert!(report.summary().contains("1 missing sound files (gone.wav)"), "{}", report.summary());
    // In the player, going over the button starts its Over sound.
    use anicraft_doc::player::{Event, NoScripts, input, start};
    let s = start(&doc, &mut NoScripts);
    let s = input(&doc, &s.state, &[Event::Move(anicraft_geom::Vec2::new(10.0, 10.0))], &mut NoScripts);
    assert_eq!(s.sounds.iter().map(|x| x.sound.sound).collect::<Vec<_>>(), [click]);
}

/// #163: Animate leaves the default stage colour (white) out of `DOMDocument.xml`, so a file
/// without `backgroundColor` has a white stage, not a black one.
#[test]
fn a_document_without_a_background_colour_has_a_white_stage() {
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> {
        Ok((p == "DOMDocument.xml").then(|| br#"<DOMDocument xmlns="http://ns.adobe.com/xfl/2008/" frameRate="12"><timelines><DOMTimeline name="Scene 1"/></timelines></DOMDocument>"#.to_vec()))
    };
    let (doc, _) = import_with(&read).unwrap();
    assert_eq!(doc.stage.background, Rgba::WHITE);
    assert_eq!((doc.stage.width, doc.stage.height), (550, 400), "Animate's default size, also left out");
}

/// ADR 0086: hostile sound data (cut short, sizes that lie, a sample count far past the file)
/// never panics, never allocates past the file, and is either read for what it holds or named
/// in the report.
#[test]
fn hostile_sound_files_are_bounded_and_named() {
    let doc = r#"<DOMDocument><media><DOMSoundItem name="s.wav" href="s.wav" sampleCount="99999999999999" format="44kHz 16bit Stereo"/></media>
      <timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="L"><frames><DOMFrame index="0" soundName="s.wav" soundLoop="4294967295"/></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"#;
    let open = |wav: Vec<u8>| {
        let files: HashMap<String, Vec<u8>> = HashMap::from([("DOMDocument.xml".to_string(), doc.as_bytes().to_vec()), ("LIBRARY/s.wav".to_string(), wav)]);
        let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(files.get(p).cloned()) };
        import_with(&read).unwrap()
    };
    let good = anicraft_doc::audio::wav::encode(8000, 2, &[0.5; 1600], anicraft_doc::audio::wav::Bits::Int16);
    let (d, _) = open(good.clone());
    assert_eq!(d.sounds[0].frames, 800);
    assert_eq!(d.timeline(0).layers[0].keyframes[0].sound.as_ref().unwrap().repeat, anicraft_doc::sound::MAX_REPEAT, "repeats bounded");
    // Every cut: read for what it holds, or named.
    for cut in 0..good.len() {
        let (d, r) = open(good[..cut].to_vec());
        assert!(d.sounds.len() == 1 || r.names.values().flatten().any(|n| n.starts_with("s.wav")), "cut at {cut}: {r:?}");
        assert!(d.sounds.iter().all(|s| s.frames <= 800));
    }
    // The data chunk claiming 4 GB, and the RIFF size lying: only what is there is read.
    let mut lying = good.clone();
    lying[40..44].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    lying[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let (d, _) = open(lying);
    assert_eq!(d.sounds[0].frames, 800);
    // 32 channels of one byte each claimed in a tiny file: no frames, so named, not imported.
    let mut wide = good;
    wide[22..24].copy_from_slice(&32u16.to_le_bytes());
    wide[32..34].copy_from_slice(&64u16.to_le_bytes());
    wide.truncate(44 + 10);
    let (d, r) = open(wide);
    assert!(d.sounds.is_empty() && r.skipped.get("sounds in formats AniCraft can't decode") == Some(&1), "{r:?}");
}

/// ADR 0086: the report names what didn't come across, a few per kind in the summary line.
#[test]
fn the_summary_names_a_few_items_and_counts_the_rest() {
    let mut media = String::new();
    for i in 0..5 {
        media.push_str(&format!(r#"<DOMSoundItem name="gone{i}.wav" href="gone{i}.wav"/>"#));
    }
    let doc = format!(r#"<DOMDocument><media>{media}</media><timelines><DOMTimeline name="Scene 1"/></timelines></DOMDocument>"#);
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok((p == "DOMDocument.xml").then(|| doc.as_bytes().to_vec())) };
    let (_, r) = import_with(&read).unwrap();
    assert_eq!(r.names["missing sound files"].len(), 5);
    assert!(r.summary().contains("5 missing sound files (gone0.wav; gone1.wav; gone2.wav and 2 more)"), "{}", r.summary());
}

/// ADR 0086: a `.fla` whose package sits in one top folder opens; one with the package at its
/// root and a folder beside it opens the root one; several documents in top folders are
/// refused with their names.
#[test]
fn a_package_in_one_top_folder_opens_and_several_are_named() {
    use crate::zip::pack::{Raw, write};
    let doc = br#"<DOMDocument width="99"><timelines><DOMTimeline name="Scene 1"/></timelines></DOMDocument>"#;
    let other = br#"<DOMDocument width="42"><timelines><DOMTimeline name="Scene 1"/></timelines></DOMDocument>"#;
    let fla = |entries: &[(&str, &[u8])]| write(&entries.iter().map(|(n, b)| Raw::new(n, b, true)).collect::<Vec<_>>(), false);
    let (d, _) = crate::import_fla(&fla(&[("Walk/DOMDocument.xml", doc), ("__MACOSX/Walk/._DOMDocument.xml", b"\0\x05\x16\x07")])).unwrap();
    assert_eq!(d.stage.width, 99);
    let (d, _) = crate::import_fla(&fla(&[("DOMDocument.xml", doc), ("Old/DOMDocument.xml", other)])).unwrap();
    assert_eq!(d.stage.width, 99, "the package at the root wins");
    let e = crate::import_fla(&fla(&[("Walk/DOMDocument.xml", doc), ("Run/DOMDocument.xml", other)])).err().unwrap().to_string();
    assert!(e.contains("2 Animate documents (Run, Walk)"), "{e}");
    // Deeper than one folder isn't looked for.
    let e = crate::import_fla(&fla(&[("a/b/DOMDocument.xml", doc)])).err().unwrap().to_string();
    assert!(e.contains("no DOMDocument.xml"), "{e}");
}
