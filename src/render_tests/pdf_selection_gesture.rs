use super::*;
use crate::view::widgets::page_selection_id;
use iced::mouse;
use iced_test::Simulator;

fn app_reading_pdf_copy(dir: &std::path::Path) -> (App, std::path::PathBuf) {
    app_reading_fixture_copy(dir, "test-multipage.pdf")
}

fn app_reading_fixture_copy(dir: &std::path::Path, fixture: &str) -> (App, std::path::PathBuf) {
    let path = dir.join("selectable.pdf");
    std::fs::copy(format!("tests/fixtures/{fixture}"), &path).unwrap();
    let mut app = app();
    // Annotation writes are refused until the native annotation scan settles.
    app.open_settled_for_test(path.clone());
    let _ = app.update(Msg::WindowResized(iced::Size::new(DESKTOP_W, DESKTOP_H)));
    (app, path)
}

fn drag_on_page(app: &mut App, (x0, y0): (f32, f32), (x1, y1): (f32, f32)) -> Vec<Msg> {
    let mut sim = Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(DESKTOP_W, DESKTOP_H),
        App::view(app),
    );
    let target = sim
        .find(iced_test::selector::id(page_selection_id(0)))
        .expect("page 0 must publish a selection layer");
    let b = target.bounds();
    let at = |x: f32, y: f32| iced::Point::new(b.x + x, b.y + y);
    let ev = |p: iced::Point| iced::Event::Mouse(mouse::Event::CursorMoved { position: p });
    // Widgets see the simulator's cursor, which only `point_at` moves, so each step points
    // before delivering the event.
    let step = |sim: &mut Simulator<'_, Msg>, p: iced::Point, e: iced::Event| {
        sim.point_at(p);
        sim.simulate([e]);
    };
    step(&mut sim, at(x0, y0), ev(at(x0, y0)));
    step(
        &mut sim,
        at(x0, y0),
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    // The first move crosses the drag threshold; the second is the band itself.
    let mid = at((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    step(&mut sim, mid, ev(mid));
    step(&mut sim, at(x1, y1), ev(at(x1, y1)));
    step(
        &mut sim,
        at(x1, y1),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    );
    let messages: Vec<Msg> = sim.into_messages().collect();
    for m in messages.clone() {
        let _ = app.update(m);
    }
    messages
}

#[test]
fn a_drag_over_the_page_is_what_supplies_the_written_rect() {
    let dir = fresh_annotation_root();
    let (mut app, pdf) = app_reading_pdf_copy(&dir);
    assert!(
        app.model().pdf_selection.is_none(),
        "nothing is selected before the drag"
    );

    // The fixture's only line of text sits in the top fifth of the page.
    let messages = drag_on_page(&mut app, (2.0, 2.0), (600.0, 220.0));
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Msg::PdfTextSelected { page: 0, range, finished: true } if !range.is_empty()
        )),
        "the drag must commit a selection from the widget's own hit-test; got {messages:?}"
    );
    let selection = app
        .model()
        .pdf_selection
        .clone()
        .expect("the committed drag must leave a live selection");
    assert_eq!(selection.page, 0);
    assert_eq!(selection.text, "Test Page 1");

    app.drive_pdf_mutation(Msg::AddPdfHighlight);
    assert_eq!(
        crate::live_feedback::announced(&app).as_deref(),
        Some("Highlight written to page 1"),
    );

    let back = crate::plumbing::pdf_ops::read_annotations(&pdf, 0).unwrap();
    assert_eq!(back.len(), 1, "exactly one annotation, on the dragged page");
    assert_eq!(back[0].contents, "");
    assert_eq!(app.model().annotations.annotations[0].text, "Test Page 1");
    let rect = back[0].rect.expect("the highlight carries a /Rect");
    assert!(
        (rect.x0 - 100.0).abs() < 0.5 && (rect.y1 - 717.23).abs() < 0.5,
        "the written /Rect must be the dragged line's, got {rect:?}"
    );
    assert!(
        (rect.x0 - 72.0).abs() > 1.0 && (rect.y0 - 680.0).abs() > 1.0,
        "the written /Rect must not be the old constant, got {rect:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_drag_over_blank_page_space_selects_nothing_and_writes_nothing() {
    let dir = fresh_annotation_root();
    let (mut app, pdf) = app_reading_pdf_copy(&dir);
    let before = std::fs::read(&pdf).unwrap();

    let messages = drag_on_page(&mut app, (2.0, 400.0), (600.0, 700.0));
    assert!(
        !messages.is_empty(),
        "the drag must still reach the selection layer"
    );
    assert!(
        app.model().pdf_selection.is_none(),
        "a band that covered no text must not arm the writers"
    );

    let _ = app.update(Msg::AddPdfHighlight);
    assert_eq!(
        crate::live_feedback::announced(&app).as_deref(),
        Some(crate::actions::SELECT_TEXT_FIRST),
    );
    assert_eq!(
        std::fs::read(&pdf).unwrap(),
        before,
        "the user's file must be untouched"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_writes_a_signed_copy_and_opens_it() {
    let dir = fresh_annotation_root();
    let (mut app, _pdf) = app_reading_fixture_copy(&dir, "multiline-prose.pdf");

    let p12 = dir.join("signer.p12");
    {
        use openssl::{asn1::Asn1Time, hash::MessageDigest, pkey::PKey, rsa::Rsa, x509};
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = x509::X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "Ada Lovelace").unwrap();
        let name = name.build();
        let mut cert = x509::X509::builder().unwrap();
        cert.set_subject_name(&name).unwrap();
        cert.set_issuer_name(&name).unwrap();
        cert.set_pubkey(&key).unwrap();
        cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        cert.set_not_after(&Asn1Time::days_from_now(30).unwrap())
            .unwrap();
        cert.sign(&key, MessageDigest::sha256()).unwrap();
        let der = openssl::pkcs12::Pkcs12::builder()
            .pkey(&key)
            .cert(&cert.build())
            .build2("pw")
            .unwrap()
            .to_der()
            .unwrap();
        std::fs::write(&p12, der).unwrap();
    }

    let _ = app.update(Msg::OpenSignDialog);
    assert_eq!(app.model().overlay, Some(crate::model::Overlay::Sign));
    let _ = app.update(Msg::SignNameChanged("Ada Lovelace".into()));
    app.model_mut().signing.as_mut().unwrap().certificate = Some(p12.clone());
    let _ = app.update(Msg::SignPasswordChanged("wrong".into()));
    let _ = crate::overlay_focus::dismiss(
        app.model_mut(),
        enzui_iced_core::modal::DismissCause::Escape,
    );
    let _ = app.update(Msg::OpenSignDialog);
    let resumed = app.model().signing.as_ref().unwrap();
    assert_eq!(resumed.name, "Ada Lovelace");
    assert_eq!(resumed.certificate.as_ref(), Some(&p12));
    assert_eq!(resumed.password, "wrong");
    let _ = app.update(Msg::PlaceSignature);
    assert!(
        app.model().signing.as_ref().unwrap().error.is_some(),
        "a wrong password is said in the dialog"
    );
    let _ = app.update(Msg::SignPasswordChanged("pw".into()));
    let _ = app.update(Msg::PlaceSignature);
    assert!(app.model().signing.as_ref().unwrap().placing);
    assert_eq!(app.model().overlay, None, "placing closes the dialog");

    let _ = app.update(Msg::SignaturePointer {
        page: 0,
        at: iced::Point::new(400.0, 900.0),
        displayed: iced::Size::new(820.0, 1061.0),
    });
    let _ = app.update(Msg::SignatureClicked(0));
    let placed = app
        .model()
        .signing
        .as_ref()
        .unwrap()
        .placed
        .expect("placed");
    assert!(
        placed.y < 300.0,
        "low on the page is near the PDF's bottom: {placed:?}"
    );

    let destination = dir.join("contract-signed.pdf");
    let task = app.write_signed_copy(destination.clone());
    for message in crate::app::drain_task(task) {
        app.drive_pdf_mutation(message);
    }
    assert!(app.model().signing.is_none(), "the flow is over");
    assert_eq!(
        app.model().file_path.as_deref(),
        Some(destination.as_path()),
        "the signed copy is what is open now"
    );
    let doc = lopdf::Document::load(&destination).unwrap();
    let page = *doc.get_pages().get(&1).unwrap();
    let (inline, referenced) = doc.get_page_resources(page).unwrap();
    let dicts = inline.into_iter().chain(
        referenced
            .iter()
            .filter_map(|id| doc.get_dictionary(*id).ok()),
    );
    let has_image = dicts
        .filter_map(|r| r.get(b"XObject").ok())
        .filter_map(|x| match x {
            lopdf::Object::Reference(id) => doc.get_dictionary(*id).ok(),
            other => other.as_dict().ok(),
        })
        .any(|x| x.iter().any(|(name, _)| name.starts_with(b"Sig")));
    assert!(has_image, "the page carries the signature image");
    let bytes = std::fs::read(&destination).unwrap();
    assert!(
        bytes.windows(19).any(|w| w == b"adbe.pkcs7.detached"),
        "the copy carries a certificate signature"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rotated_pages_select_and_paint_the_same_text_in_every_quarter_turn() {
    let dir = fresh_annotation_root();
    for quarter in 0..4 {
        let (mut app, _) = app_reading_pdf_copy(&dir);
        app.model_mut().rotation = crate::viewport::PageRotation::from_quarters(quarter);
        app.model_mut().zoom.fit_page();
        let bounds = {
            let mut sim = Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(DESKTOP_W, DESKTOP_H),
                app.view(),
            );
            sim.find(iced_test::selector::id(page_selection_id(0)))
                .expect("rotated page selection layer")
                .bounds()
        };
        let (width, height) = (bounds.width, bounds.height);
        let text = &app.model().pdf_text[0];
        let glyphs = text.glyphs.glyphs();
        let [x, y, _, h] = glyphs[0].rect;
        let last = glyphs.last().unwrap().rect;
        let turn = |x: f32, y: f32| {
            let (x, y) = (x / text.width_pts, y / text.height_pts);
            let (x, y) = match quarter {
                1 => (1.0 - y, x),
                2 => (1.0 - x, 1.0 - y),
                3 => (y, 1.0 - x),
                _ => (x, y),
            };
            (x * width, y * height)
        };
        let start = turn(x - 1.0, y + h / 2.0);
        let end = turn(last[0] + last[2] + 1.0, last[1] + last[3] / 2.0);
        let before = frame_of_sized(&app, DESKTOP_W, DESKTOP_H).0;
        let blue_changes = |before: &[u8], after: &[u8]| {
            let mut changed = 0;
            for y in bounds.y.max(0.0) as u32..(bounds.y + bounds.height).min(DESKTOP_H) as u32 {
                for x in bounds.x.max(0.0) as u32..(bounds.x + bounds.width).min(DESKTOP_W) as u32 {
                    let i = ((y * DESKTOP_W as u32 + x) * 4) as usize;
                    let a = &before[i..i + 3];
                    let b = &after[i..i + 3];
                    if a != b && i32::from(b[2]) > i32::from(b[0]) + 15 {
                        changed += 1;
                    }
                }
            }
            changed
        };
        let unchanged = frame_of_sized(&app, DESKTOP_W, DESKTOP_H).0;
        assert_eq!(
            blue_changes(&before.0, &unchanged.0),
            0,
            "unchanged page control"
        );
        let messages = drag_on_page(&mut app, start, end);
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Msg::PdfTextSelected { finished: true, .. })),
            "quarter {quarter}: {messages:?}"
        );
        assert_eq!(
            app.model().pdf_selection.as_ref().unwrap().text,
            "Test Page 1",
            "quarter {quarter}"
        );
        let actions: Vec<_> = nodes(&app)
            .into_iter()
            .filter(|node| {
                node.semantics.label.as_deref().is_some_and(|label| {
                    node.id
                        == Some(enzui_iced_core::identity::derive_id(
                            "reader.selection",
                            label,
                        ))
                })
            })
            .collect();
        assert_eq!(actions.len(), 4, "selection actions positive control");
        let left = actions
            .iter()
            .map(|n| n.bounds.x)
            .fold(f32::INFINITY, f32::min);
        let right = actions
            .iter()
            .map(|n| n.bounds.x + n.bounds.width)
            .fold(0.0, f32::max);
        let expected_centre = (bounds.x + (start.0 + end.0) / 2.0).clamp(80.0, DESKTOP_W - 80.0);
        assert!(
            ((left + right) / 2.0 - expected_centre).abs() < 60.0,
            "quarter {quarter}: popup {left}..{right} is detached from selection centre {expected_centre}"
        );
        let _ = app.update(Msg::DismissPdfSelectionActions);
        let after = frame_of_sized(&app, DESKTOP_W, DESKTOP_H).0;
        let changed = blue_changes(&before.0, &after.0);
        assert!(
            changed > 30,
            "quarter {quarter}: missing native selection paint ({changed} pixels)"
        );
        use enzui_iced_core::shortcut::{Modifier, Shortcut};
        use iced_test::futures::futures::{FutureExt, StreamExt};
        let event = crate::shortcuts::event_for_shortcut(&Shortcut::new(Modifier::CTRL, 'c'));
        assert!(crate::app::classify_event_in(&event, iced::event::Status::Captured).is_none());
        let message = crate::app::classify_event_in(&event, iced::event::Status::Ignored)
            .expect("copy shortcut");
        let task = app.update(message);
        let mut stream = iced_test::runtime::task::into_stream(task).expect("copy task");
        let action = stream
            .next()
            .now_or_never()
            .flatten()
            .expect("clipboard write");
        let iced_test::runtime::Action::Clipboard(iced_test::runtime::clipboard::Action::Write {
            content,
            ..
        }) = action
        else {
            panic!("copy shortcut did not reach the clipboard");
        };
        let iced::advanced::clipboard::Content::Text(text) = content else {
            panic!("expected text");
        };
        assert_eq!(text, "Test Page 1", "quarter {quarter}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
