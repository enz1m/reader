use super::*;

#[test]
fn remove_from_recents_drops_the_book_and_persists_it() {
    let dir = tmpdir("recents");
    let resolver = crate::app::test_xdg_resolver(&dir);
    let book = dir.join("b.pdf");
    std::fs::write(&book, b"%PDF-1.4\n").unwrap();

    let (mut app, _t) = App::test_new();
    app.enable_persist(resolver.clone());
    persistence::set_progress(
        std::sync::Arc::make_mut(&mut app.model.app_state),
        &book.to_string_lossy(),
        persistence::ProgressRecord {
            page: 0,
            title: "B".to_string(),
            format: "pdf".to_string(),
            ..Default::default()
        },
    );
    super::refresh_projection(&mut app.model);
    assert_eq!(app.model.recents.len(), 1);

    let _ = app.update(Msg::RemoveFromRecents(book.to_string_lossy().into_owned()));
    assert!(app.model.recents.is_empty(), "the card is gone");

    let reloaded = persistence::load_state_with_resolver(&resolver);
    assert!(
        persistence::get_progress(&reloaded, &book.to_string_lossy()).is_none(),
        "the removal must survive a restart"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
