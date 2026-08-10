import json

import pytest

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


def test_estimator_cache_rejects_abi_mismatch_and_schema_v1(tmp_path) -> None:
    path = tmp_path / "state.json"
    save_estimator_state('{"version":8}', native_abi="abi-1", path=path)

    assert load_estimator_state(native_abi="abi-2", path=path) is None

    path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "native_abi": "abi-1",
                "estimator_state_json": '{"version":8}',
            }
        ),
        encoding="utf-8",
    )
    assert load_estimator_state(native_abi="abi-1", path=path) is None


@pytest.mark.parametrize("state_json", ["[]", "not-json"])
def test_estimator_cache_rejects_non_object_inner_state(tmp_path, state_json: str) -> None:
    path = tmp_path / "state.json"
    path.write_text(
        json.dumps(
            {
                "schema_version": 2,
                "native_abi": "abi-1",
                "estimator_state_json": state_json,
            }
        ),
        encoding="utf-8",
    )

    assert load_estimator_state(native_abi="abi-1", path=path) is None


def test_estimator_cache_identity_ignores_build_commit_and_fps(tmp_path) -> None:
    path = tmp_path / "state.json"
    save_estimator_state('{"version":8}', native_abi="abi-1", path=path)
    payload = json.loads(path.read_text(encoding="utf-8"))
    payload["unrelated_build_commit"] = "different-build"
    payload["fps"] = 144
    path.write_text(json.dumps(payload), encoding="utf-8")

    assert load_estimator_state(native_abi="abi-1", path=path) == '{"version":8}'


def test_estimator_cache_save_failure_is_non_fatal(tmp_path, monkeypatch) -> None:
    path = tmp_path / "state.json"

    def fail_replace(*args, **kwargs):
        raise OSError("simulated replace failure")

    monkeypatch.setattr("sky_music.orchestration.estimator_cache.os.replace", fail_replace)
    save_estimator_state('{"version":8}', native_abi="abi-1", path=path)

    assert not path.exists()
