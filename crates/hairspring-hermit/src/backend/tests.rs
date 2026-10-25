use super::inconclusive_text;

#[test]
fn the_inconclusive_note_keeps_the_spawn_error_text() {
    let text = inconclusive_text(2, Some("cannot start hermit: no such file"));
    assert!(text.contains("2 reruns inconclusive"), "{text}");
    assert!(text.contains("cannot start hermit: no such file"), "{text}");
}

#[test]
fn the_inconclusive_note_without_a_spawn_error_is_unchanged() {
    assert_eq!(
        inconclusive_text(1, None),
        "; 1 rerun inconclusive (host or infrastructure, not the program)"
    );
}
