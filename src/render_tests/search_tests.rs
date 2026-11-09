use super::*;

#[test]
fn enter_in_the_find_field_steps_to_the_next_match() {
    use crate::view::search_panel::match_counter;
    use iced_test::Simulator;
    use iced_test::selector::{Candidate, Target};

    let mut app = app_searching("coast");
    assert!(
        app.model().search_hits.len() >= 2,
        "need at least two hits to step between"
    );
    assert_eq!(
        app.model().search_active,
        0,
        "search starts on the first hit"
    );
    let before = match_counter(app.model());

    let mut sim = Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(DESKTOP_W, DESKTOP_H),
        App::view(&app),
    );
    // Selected by its live content, so a header or result row containing the word cannot match.
    sim.click(|c: Candidate<'_>| match c {
        Candidate::TextInput { state, .. } if state.text() == "coast" => Some(Target::from(c)),
        _ => None,
    })
    .expect("the find field must be on screen with the query in it");
    let _ = sim.tap_key(iced::keyboard::key::Named::Enter);

    let messages: Vec<Msg> = sim.into_messages().collect();
    assert!(
        messages.iter().any(|m| matches!(m, Msg::NextMatch)),
        "Enter in the query field must emit NextMatch; got {messages:?}"
    );
    for m in messages {
        let _ = app.update(m);
    }

    assert_eq!(
        app.model().search_active,
        1,
        "Enter must advance the active match"
    );
    let after = match_counter(app.model());
    assert!(
        after.starts_with("2 of "),
        "the counter must read hit 2, got {after:?}"
    );
    let ns = nodes(&app);
    assert_eq!(
        role_count(&ns, Role::Status, &after),
        1,
        "the advanced counter {after:?} must be on screen"
    );
    assert_eq!(
        role_count(&ns, Role::Status, &before),
        0,
        "the stale counter {before:?} must be gone"
    );
}
