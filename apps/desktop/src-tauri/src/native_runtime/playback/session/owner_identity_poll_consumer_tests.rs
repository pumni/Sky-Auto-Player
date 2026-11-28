use super::consume_owner_identity_poll_result;
use sky_player::engine::OwnerIdentityPollResult;

#[test]
fn current_valid_owner_poll_does_not_panic() {
    let mut invalidated = false;
    let mut panic_release_calls = 0;
    consume_owner_identity_poll_result(
        OwnerIdentityPollResult::CurrentValid,
        &mut invalidated,
        || panic_release_calls += 1,
    );

    assert!(!invalidated);
    assert_eq!(panic_release_calls, 0);
}

#[test]
fn current_invalid_owner_poll_panics_once_even_if_polled_again() {
    let mut invalidated = false;
    let mut panic_release_calls = 0;
    for _ in 0..2 {
        consume_owner_identity_poll_result(
            OwnerIdentityPollResult::CurrentInvalid,
            &mut invalidated,
            || panic_release_calls += 1,
        );
    }

    assert!(invalidated);
    assert_eq!(panic_release_calls, 1);
}
