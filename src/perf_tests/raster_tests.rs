use super::*;

#[test]
fn large_and_encrypted_pdfs_keep_a_bounded_resident_working_set() {
    {
        let (mut app, _t) = App::test_new();
        open_long_pdf(&mut app);
        let budget = app.model().pages.budget();
        assert_eq!(
            budget,
            crate::raster::RASTER_BUDGET_BYTES,
            "the gate must run against the SHIPPED budget, not a test one"
        );

        // Bands do not overlap, so the walk touches ~80 distinct pages, four times the budget.
        let step = 16;
        let mut touched = 0usize;
        let mut peak_bytes = 0usize;
        let mut peak_rss = rss_kb();
        let mut forward: Vec<usize> = (0..LONG_PDF_PAGES).step_by(step).collect();
        forward.push(LONG_PDF_PAGES - 1);
        let mut sweep = forward.clone();
        sweep.extend(forward.iter().rev().copied());
        for target in sweep {
            jump_and_settle(&mut app, target);
            touched += 1;
            peak_bytes = peak_bytes.max(app.model().pages.bytes());
            peak_rss = peak_rss.max(rss_kb());
            assert!(
                app.model().pages.bytes() <= budget,
                "after jumping to page {}, {} bytes were resident over a {budget}-byte \
             budget ({} pages: {:?})",
                target + 1,
                app.model().pages.bytes(),
                app.model().pages.len(),
                app.model().pages.resident()
            );
        }
        println!(
            "long-PDF sweep: {touched} jumps over {LONG_PDF_PAGES} pages; peak resident \
         {peak_bytes} B of {budget} B budget in {} pages; {} evictions; peak process RSS {peak_rss} kB",
            app.model().pages.len(),
            app.model().pages.evictions()
        );
        assert!(
            app.model().pages.evictions() > 0,
            "a sweep of a 400-page document evicted nothing — the bound is a \
         ceiling nothing has ever reached, which is how it stops being true"
        );
        assert!(
            !app.model().pages.contains(LONG_PDF_PAGES - 1),
            "back at the start of the document, page {LONG_PDF_PAGES}'s raster must \
         not still be held: resident {:?}",
            app.model().pages.resident()
        );
        assert!(
            app.model().pages.contains(0),
            "the page the reader is looking at must be resident: {:?}",
            app.model().pages.resident()
        );

        // At the far end the first page is the one evicted.
        jump_and_settle(&mut app, LONG_PDF_PAGES - 1);
        assert!(
            !app.model().pages.contains(0),
            "at page {LONG_PDF_PAGES}, page 1's raster must have been evicted: {:?}",
            app.model().pages.resident()
        );
        assert!(
            app.model().pages.len() < 64,
            "the working set is {} pages — it must follow the viewport, not the walk",
            app.model().pages.len()
        );
    }
    {
        let dir = tmpdir("locked-lazy");
        let pdf = dir.join("locked-60.pdf");
        build_pdf(&pdf, PDF_PAGES, Some(PDF_PASSWORD));

        let (mut app, _t) = App::test_new();
        let _ = app.update(Msg::WindowResized(Size::new(W, H)));
        app.arm_password_prompt_for_test(&pdf, PDF_PASSWORD);
        for message in crate::app::drain_task(app.update(Msg::SubmitPassword)) {
            for follow_up in crate::app::drain_task(app.update(message)) {
                let _ = app.update(follow_up);
            }
        }
        assert_eq!(
            app.model().page_count,
            PDF_PAGES,
            "the unlocked document must be its full length"
        );
        assert!(
            app.model().pdf_password.is_some(),
            "the session key must outlive the unlock, or no later page can be served"
        );
        assert!(
            app.model().password_input.is_empty() && app.model().password_prompt.is_none(),
            "the key is retained in ONE field; the dialog's copy is still wiped"
        );

        // Far outside the unlock's band, so a fresh poppler open needs the key.
        jump_and_settle(&mut app, 45);
        assert!(
            app.model().pages.contains(45),
            "page 46 of the encrypted document must rasterise after the unlock; \
         resident {:?}, failed {:?}",
            app.model().pages.resident(),
            app.model().raster_failed
        );
        assert!(
            app.model().raster_failed.is_empty(),
            "no page of an unlocked document may be unavailable: {:?}",
            app.model().raster_failed
        );

        open_long_pdf(&mut app);
        assert!(
            app.model().pdf_password.is_none(),
            "the session key must die with the document it unlocked"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// Lazy bands reopen the file because Poppler documents are !Send; the key must survive until close.
