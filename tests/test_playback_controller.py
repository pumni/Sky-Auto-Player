from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.domain import Song, Note, NoteKey, Millis
from sky_music.ui.textual_app.playback_controller import (
    PlaybackPlan,
    PlaybackError,
    prepare_playback,
    rebuild_with,
)

def test_prepare_playback_success() -> None:
    # A clean simple song
    song = Song(
        name="Test Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(100), key=NoteKey("Key1")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    plan = prepare_playback(song, session, cfg)
    assert isinstance(plan, PlaybackPlan)
    assert plan.song == song
    assert plan.session == session
    assert len(plan.actions) > 0
    assert plan.risk_report is not None

def test_prepare_playback_build_failed() -> None:
    # A song with an invalid key that cannot be resolved to a scan code
    song = Song(
        name="Invalid Key Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("KeyInvalid")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    error = prepare_playback(song, session, cfg)
    assert isinstance(error, PlaybackError)
    assert error.code == "build_failed"
    assert "KeyInvalid" in error.message

def test_prepare_playback_validation_failed() -> None:
    # A song with negative timestamp to trigger a fatal validation violation
    song = Song(
        name="Negative Time Song",
        notes=(
            Note(time_ms=Millis(-50), key=NoteKey("Key0")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    error = prepare_playback(song, session, cfg, is_dry_run=False)
    assert isinstance(error, PlaybackError)
    assert error.code == "validation_failed"
    assert "negative_timestamp" in error.message

def test_rebuild_with_plan() -> None:
    song = Song(
        name="Test Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    plan = prepare_playback(song, session, cfg)
    assert isinstance(plan, PlaybackPlan)

    # Rebuild plan with new profile and tempo
    rebuilt = rebuild_with(plan, profile="audience-safe", tempo=0.9)
    assert isinstance(rebuilt, PlaybackPlan)
    assert rebuilt.session.profile_name == "audience-safe"
    assert rebuilt.session.tempo_scale == 0.9

def test_rebuild_with_session() -> None:
    song = Song(
        name="Test Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    # Rebuild session with new profile and tempo
    rebuilt = rebuild_with(session, cfg=cfg, song=song, profile="audience-safe", tempo=0.9)
    assert isinstance(rebuilt, PlaybackPlan)
    assert rebuilt.session.profile_name == "audience-safe"
    assert rebuilt.session.tempo_scale == 0.9

def test_prepare_playback_high_risk() -> None:
    # A song with high-risk same-key repeats spaced extremely close (2ms)
    song = Song(
        name="High Risk Song",
        notes=(
            Note(time_ms=Millis(0), key=NoteKey("Key0")),
            Note(time_ms=Millis(2), key=NoteKey("Key0")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    plan = prepare_playback(song, session, cfg, is_dry_run=True)
    assert isinstance(plan, PlaybackPlan)
    assert plan.risk_report.severity == "high"

def test_prepare_playback_dry_run_with_violations() -> None:
    # A song with negative timestamp under dry-run should return PlaybackPlan and keep violations
    song = Song(
        name="Violations Song",
        notes=(
            Note(time_ms=Millis(-50), key=NoteKey("Key0")),
        ),
    )
    session = PlaybackSessionContext.balanced(tempo_scale=1.0)
    cfg = AppConfig()

    plan = prepare_playback(song, session, cfg, is_dry_run=True)
    assert isinstance(plan, PlaybackPlan)
    assert len(plan.violations) > 0
    assert any(v.code == "negative_timestamp" for v in plan.violations)
