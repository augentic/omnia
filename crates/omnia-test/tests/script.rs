//! Contract of the shared `Script` core: FIFO order, recording, and the
//! three exhaustion behaviours (panic, fallback, drop check).

use std::panic::{AssertUnwindSafe, catch_unwind};

use omnia_test::Script;

fn message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

#[test]
fn pops_in_order_and_records_requests() {
    let script = Script::new(["one", "two"]);
    assert_eq!(script.next("first"), "one");
    assert_eq!(script.next("second"), "two");
    assert_eq!(script.seen(), ["first", "second"]);
    assert_eq!(script.remaining(), 0);
    script.assert_exhausted();
}

#[test]
fn clones_share_the_queue() {
    let script = Script::new([1, 2]);
    let handle = script.clone();
    assert_eq!(handle.next(()), 1);
    assert_eq!(script.next(()), 2);
    assert_eq!(script.seen().len(), 2);
    script.assert_exhausted();
}

#[test]
fn consuming_past_the_end_panics() {
    let script = Script::<&str, i32>::new([1]);
    assert_eq!(script.next("a"), 1);
    let result = catch_unwind(AssertUnwindSafe(|| script.next("b")));
    let text = message(&*result.expect_err("exhausted script panics"));
    assert!(text.contains("script exhausted"), "{text}");
    assert!(text.contains("1 turn(s) consumed"), "{text}");
    assert!(text.contains("request #2"), "{text}");
}

#[test]
fn then_answers_past_the_end() {
    let script = Script::new([1]).then(|| -1);
    assert_eq!(script.next(()), 1);
    assert_eq!(script.next(()), -1);
    assert_eq!(script.next(()), -1);
    assert_eq!(script.seen().len(), 3);
}

#[test]
fn edit_changes_a_pending_turn() {
    let script = Script::<(), Vec<&str>>::new([vec![], vec!["b"]]).edit(1, |turn| turn.push("c"));
    assert_eq!(script.next(()), Vec::<&str>::new());
    assert_eq!(script.next(()), ["b", "c"]);
}

#[test]
fn edit_out_of_range_panics() {
    let result = catch_unwind(|| Script::<(), i32>::new([1]).edit(3, |_| {}));
    let text = message(&*result.expect_err("editing a missing turn panics"));
    assert!(text.contains("no scripted turn at index 3"), "{text}");
}

#[test]
fn assert_exhausted_names_the_remainder() {
    let script = Script::<(), i32>::new([1, 2, 3]);
    assert_eq!(script.next(()), 1);
    let result = catch_unwind(AssertUnwindSafe(|| script.assert_exhausted()));
    let text = message(&*result.expect_err("unconsumed turns fail the assertion"));
    assert!(text.contains("2 unconsumed turn(s)"), "{text}");
    // Consume the rest so the drop check below does not fire a second time.
    script.next(());
    script.next(());
}

#[test]
fn dropping_with_turns_left_panics() {
    let result = catch_unwind(|| {
        let script = Script::<(), i32>::new([1, 2]);
        script.next(());
    });
    let text = message(&*result.expect_err("the last handle dropping with a turn left panics"));
    assert!(text.contains("dropped with 1 unconsumed turn(s)"), "{text}");
}

#[test]
fn try_next_records_an_overrun_instead_of_panicking() {
    let script = Script::<&str, i32>::new([1]);
    assert_eq!(script.try_next("a"), Some(1));
    assert_eq!(script.try_next("b"), None);
    assert_eq!(script.seen(), ["a", "b"], "the overrunning request is still recorded");
    assert_eq!(script.overruns(), 1);
    let result = catch_unwind(AssertUnwindSafe(|| script.assert_exhausted()));
    let text = message(&*result.expect_err("an overrun fails the assertion"));
    assert!(text.contains("1 request(s) past the end"), "{text}");
}

#[test]
fn then_answers_try_next_without_an_overrun() {
    let script = Script::<(), i32>::new([]).then(|| 7);
    assert_eq!(script.try_next(()), Some(7));
    assert_eq!(script.overruns(), 0);
    script.assert_exhausted();
}

#[test]
fn dropping_with_an_overrun_panics() {
    let result = catch_unwind(|| {
        let script = Script::<(), i32>::new([]);
        assert_eq!(script.try_next(()), None);
    });
    let text = message(&*result.expect_err("the last handle dropping after an overrun panics"));
    assert!(text.contains("1 request(s) past the end"), "{text}");
}

#[test]
fn drop_check_stays_silent_once_asserted() {
    let script = Script::<(), i32>::new([1, 2]);
    let result = catch_unwind(AssertUnwindSafe(|| script.assert_exhausted()));
    assert!(result.is_err(), "the assertion reports the remainder");
    drop(script);
}

#[test]
fn drop_check_stays_silent_under_an_unrelated_panic() {
    let result = catch_unwind(|| {
        let _script = Script::<(), i32>::new([1]);
        panic!("the real failure");
    });
    let text = message(&*result.expect_err("the scenario's own panic propagates"));
    assert_eq!(text, "the real failure");
}
