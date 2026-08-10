import json

from sky_music.orchestration.estimator_cache import (
    load_estimator_state,
    save_estimator_state,
)


def test_estimator_cache_round_trips_atomically(tmp_path) -> None:
    path = tmp_path / "state.json"
    save_estimator_state(
        json.dumps({"version": 8}),
        native_abi="abi-1",
        path=path,
    )
    assert load_estimator_state(
        native_abi="abi-1",
        path=path,
    ) == '{"version": 8}'


def test_estimator_cache_rejects_corrupt_or_mismatched_state(tmp_path) -> None:
    path = tmp_path / "state.json"
    path.write_text("not json", encoding="utf-8")
    assert load_estimator_state(
        native_abi="abi-1",
        path=path,
    ) is None

    save_estimator_state(
        "{}",
        native_abi="abi-1",
        path=path,
    )
    assert load_estimator_state(path=path, native_abi="abi-1") is not None

    payload = json.loads(path.read_text(encoding="utf-8"))
    assert set(payload) == {"schema_version", "native_abi", "estimator_state_json"}
