//! Crate-level tests: the documentation matches the program, and the
//! commands and the GPU plugin work end to end on a synthetic study.

use crate::cli::{Backend, Cli};
use crate::commands::{self, filter_tags, is_linked, open_study};
use crate::gpu::{ImagePlugin, Look, WgpuPlugin};
use clap::CommandFactory;
use dicomscope_core::dicom::testutil::Synthetic;
use std::path::PathBuf;

const README: &str = include_str!("../README.md");
const SAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples");

/// A one-file study on disk, built in memory: the DICOM samples are not
/// committed, the HL7 orders are.
fn synthetic_study(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dicomscope-tui-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("1.dcm");
    std::fs::write(&path, Synthetic::default().build()).unwrap();
    path
}

#[test]
fn readme_documents_every_command_and_key() {
    let plain = README.replace('`', "");
    for sub in Cli::command().get_subcommands() {
        let name = sub.get_name();
        assert!(
            README.contains(&format!("### {name}")),
            "README has no section for `{name}`"
        );
    }
    for (keys, _) in crate::ui::KEYS {
        for key in keys.split([',', ' ']).filter(|k| !k.is_empty()) {
            let key = key.trim_start_matches('/').trim();
            if key.is_empty() || key == "wheel" {
                continue;
            }
            assert!(plain.contains(key), "README does not mention key {key:?}");
        }
    }
}

#[test]
fn tags_filter_ignores_case() {
    let s = open_study(&[synthetic_study("tags")]).unwrap();
    let all = filter_tags(&s.tags, None).len();
    let some = filter_tags(&s.tags, Some("ROWS")).len();
    assert!(all > some && some >= 1, "{all} rows, {some} matching");
}

#[test]
fn study_and_order_link_and_produce_fhir() {
    let mut s = open_study(&[synthetic_study("link")]).unwrap();
    s.open_hl7(&format!("{SAMPLES}/order.hl7"));
    assert!(s.error.is_none(), "{:?}", s.error);
    let l = s.linkage.clone().unwrap();
    let text = commands::link_text(&l, s.chain.as_ref());
    assert!(text.starts_with(if is_linked(&l) {
        "LINKED"
    } else {
        "NOT LINKED"
    }));
    assert!(text.contains("Study Instance UID"));
    assert!(s.fhir.as_ref().unwrap().bundle.contains("\"Bundle\""));
}

#[test]
fn a_missing_series_is_an_error_not_a_panic() {
    let a = crate::cli::TagsArgs {
        study: vec![synthetic_study("series")],
        at: crate::cli::SliceArgs {
            series: 9,
            slice: 1,
        },
        grep: None,
        format: Default::default(),
    };
    let e = commands::tags(&a).unwrap_err();
    assert!(e.contains("series 9 does not exist"), "{e}");
}

/// Real GPU work: skipped where no adapter exists (headless CI without
/// a software rasteriser), run everywhere else.
#[test]
fn the_wgpu_plugin_renders_the_slice_offscreen() {
    let mut plugin = match WgpuPlugin::new(Backend::Auto) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipped, no GPU adapter: {e}");
            return;
        }
    };
    let mut s = open_study(&[synthetic_study("gpu")]).unwrap();
    let size = (30, 20);
    s.set_canvas(size);
    let img = s.image().unwrap();
    s.view = s.view.fit(size, img);
    let picture = plugin.render(&mut s, size, Look::default()).unwrap();
    assert_eq!(picture.dimensions(), size);
    // The 3x2 synthetic image is fitted into 30x20 with bars left and
    // right; the centre is image, the corners black.
    let lit = |x, y| picture.get_pixel(x, y).0[..3].iter().any(|c| *c > 0);
    assert!(!lit(0, 0), "corner is outside the image");
    let inside = (1..19).any(|y| (1..29).any(|x| lit(x, y)));
    assert!(inside, "no pixel of the image was drawn");
    let inverted = plugin.render(&mut s, size, Look { invert: true }).unwrap();
    assert_ne!(picture, inverted, "inversion changes the picture");
    assert!(plugin.describe().contains(" · "));
}

/// The viewer end to end on a test terminal: open a study and an order,
/// press keys, visit every panel. Half blocks, so the image lands in the
/// buffer as coloured cells and can be checked.
#[test]
fn the_viewer_draws_every_panel_and_follows_keys() {
    use crate::app::{App, Tab};
    use crate::cli::{GpuArgs, ViewArgs};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui_image::picker::Picker;

    let args = ViewArgs {
        paths: vec![
            synthetic_study("viewer"),
            PathBuf::from(format!("{SAMPLES}/order.hl7")),
        ],
        series: 1,
        slice: 1,
        ..Default::default()
    };
    let gpu = GpuArgs {
        backend: Backend::Auto,
    };
    let mut app = App::new(&args, gpu, crate::app::Mode::Terminal(Picker::halfblocks()));
    let mut term = ratatui::Terminal::new(TestBackend::new(120, 36)).unwrap();
    let key = |c: KeyCode| Event::Key(KeyEvent::new(c, KeyModifiers::NONE));
    let screen = |t: &ratatui::Terminal<TestBackend>| {
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| {
                (0..b.area.width)
                    .map(|x| b[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    app.frame(&mut term).unwrap();
    let s = screen(&term);
    assert!(s.contains("F1 Image") && s.contains("F7 Documents"), "{s}");
    assert!(s.contains("series 1/1"), "{s}");
    assert!(s.contains("ORM^O01"), "order summary in the sidebar:\n{s}");
    if app.plugin.is_ok() {
        assert!(
            app.graphic.is_some(),
            "the slice was rendered: {:?}",
            app.render_error
        );
        assert!(
            s.contains('▀') || s.contains('▄'),
            "half blocks drawn:\n{s}"
        );
    }

    let before = app.session.view.scale;
    for c in ['-', 'r', 'w', 'i', 'e'] {
        app.handle(key(KeyCode::Char(c)));
        app.frame(&mut term).unwrap();
    }
    // Zoom out: the 3x2 test image already sits at the maximum scale.
    assert!(app.session.view.scale < before);
    assert_eq!(app.session.view.rotation, 1);
    assert!(app.look.invert && app.session.enhance);

    for tab in Tab::ALL.iter().skip(1) {
        app.handle(key(KeyCode::Tab));
        assert_eq!(app.tab, *tab);
        app.frame(&mut term).unwrap();
        let s = screen(&term);
        assert!(s.contains(tab.title()), "{:?} panel:\n{s}", tab);
    }
    // FHIR: the Bundle, then the next resource.
    app.handle(key(KeyCode::F(6)));
    app.frame(&mut term).unwrap();
    assert!(screen(&term).contains("\"resourceType\""));
    app.handle(key(KeyCode::Right));
    app.frame(&mut term).unwrap();
    assert!(screen(&term).contains("◀ Patient ▶"));

    // Tags: filter by typing.
    app.handle(key(KeyCode::F(2)));
    for c in "rows".chars() {
        if app.filtering {
            app.handle(key(KeyCode::Char(c)));
        } else {
            app.handle(key(KeyCode::Char('/')));
            app.handle(key(KeyCode::Char(c)));
        }
    }
    app.handle(key(KeyCode::Enter));
    app.frame(&mut term).unwrap();
    let s = screen(&term);
    assert!(s.contains("/rows") && s.contains("Rows"), "{s}");

    app.handle(key(KeyCode::Char('?')));
    app.frame(&mut term).unwrap();
    assert!(screen(&term).contains("Keys"));
    app.handle(key(KeyCode::Char('x')));
    assert!(!app.help);

    app.handle(key(KeyCode::Char('q')));
    assert!(app.quitting());
}
