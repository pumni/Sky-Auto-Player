"""Rendering helpers for Textual UI."""

from __future__ import annotations

from pathlib import Path

from rich.text import Text

from sky_music.ui.picker_helpers import SONG_DIR, SUPPORTED_EXTENSIONS
from sky_music.ui.picker_metadata import SongUiMetadata
from sky_music.ui.picker_theme import ThemePreset, get_match_span
from sky_music.ui.text_render import cell_width, truncate_cells
from sky_music.ui.textual_app.theme_css import TextualThemeTokens

UNKNOWN_FIELD = "-"
PENDING_FIELD = "..."


def _title_cell(title: str, normalized_query: str, match_style: str = "bold #fbbf24") -> Text:
    text = Text(title)
    if not normalized_query:
        return text
    span = get_match_span(title, normalized_query)
    if span is None:
        return text
    start, end = span
    text.stylize(match_style, start, end)
    return text


def _risk_style(risk: str, muted: str, theme: ThemePreset) -> str:
    risk_upper = risk.upper()
    if theme.name == "classic":
        if risk_upper in ("LOW", "SUCCESS"):
            return theme.foreground
        if risk_upper in ("MED", "MEDIUM", "WARN", "WARNING"):
            return f"bold {theme.foreground}"
        if risk_upper in ("HIGH", "DANGER", "ERROR"):
            return f"bold reverse {theme.foreground}"
        return muted

    if risk_upper in ("LOW", "SUCCESS"):
        return f"bold {theme.success}"
    if risk_upper in ("MED", "MEDIUM", "WARN", "WARNING"):
        return f"bold {theme.warning}"
    if risk_upper in ("HIGH", "DANGER", "ERROR"):
        return f"bold {theme.danger}"
    return muted


def _risk_cell(risk: str, muted: str, theme: ThemePreset) -> Text:
    """Colour-code the difficulty/risk column using theme semantic colors/styles."""
    return Text(risk, style=_risk_style(risk, muted, theme))


def _format_duration(seconds: float) -> str:
    total_seconds = max(0, round(seconds))
    minutes, sec = divmod(total_seconds, 60)
    return f"{minutes}:{sec:02d}"


def _metadata_cells(metadata: SongUiMetadata | None) -> tuple[str, str, str, str]:
    if metadata is None:
        return UNKNOWN_FIELD, UNKNOWN_FIELD, UNKNOWN_FIELD, "loading"

    duration = _format_duration(metadata.duration_seconds)
    notes = str(metadata.note_count)
    if not metadata.analyzed:
        return duration, notes, PENDING_FIELD, PENDING_FIELD
    return duration, notes, metadata.risk.upper(), metadata.recommended_profile


def _warning_summary(warnings: tuple[str, ...], *, max_width: int = 72) -> str:
    if not warnings:
        return ""
    first = " ".join(warnings[0].split())
    suffix = f"  +{len(warnings) - 1} more" if len(warnings) > 1 else ""
    return truncate_cells(first, max(8, max_width - cell_width(suffix))) + suffix


def build_empty_detail_text(t: TextualThemeTokens, has_choices: bool, query: str) -> Text:
    txt = Text()
    if not has_choices:
        supported = ", ".join(sorted(SUPPORTED_EXTENSIONS))
        txt.append(f"No songs found in {SONG_DIR}", style=f"bold {t.foreground}")
        txt.append("\n")
        txt.append(f"Supported: {supported}", style=t.muted)
        txt.append("\n")
        txt.append("Press Ctrl+R to reload", style=t.muted)
    elif query.strip():
        txt.append(f'No matches for "{query.strip()}"', style=f"bold {t.foreground}")
        txt.append("\n")
        txt.append("Clear search or press Ctrl+R to reload", style=t.muted)
    else:
        txt.append("No song selected", style=t.muted)
    return txt


def build_detail_text(selected_path: Path, metadata: SongUiMetadata | None, t: TextualThemeTokens) -> Text:
    if metadata is None:
        txt = Text()
        txt.append(selected_path.stem, style=f"bold {t.foreground}")
        txt.append("\n")
        txt.append("analyzing…", style=t.muted)
        return txt

    analyzed = metadata.analyzed
    risk = metadata.risk.upper() if analyzed else "…"
    suggested = metadata.recommended_profile if analyzed else "…"
    risk_style = _risk_style(risk, t.muted, t) if analyzed else t.muted

    def label(s: str) -> tuple[str, str]:
        return (s, t.accent_dim)

    def value(s: str, style: str | None = None) -> tuple[str, str]:
        return (s, style or t.foreground)

    txt = Text()
    txt.append(selected_path.stem, style=f"bold {t.foreground}")
    txt.append("\n")
    txt.append_text(Text.assemble(
        label("time "), value(_format_duration(metadata.duration_seconds), t.accent),
        label("  notes "), value(str(metadata.note_count)),
        label("  risk "), value(risk, risk_style),
    ))
    txt.append("\n")
    txt.append_text(Text.assemble(
        label("suggested "), value(suggested, t.accent),
        label("  tempo "), value(f"{metadata.recommended_tempo_scale:.2f}×"),
    ))
    txt.append("\n")
    txt.append_text(Text.assemble(
        label("avg "), value(f"{metadata.average_notes_per_second:.1f}/s"),
        label("  peak "), value(f"{metadata.peak_notes_per_second_1s:.1f}/s"),
        label("  chords "), value(str(metadata.chords_count)),
    ))
    txt.append("\n")
    txt.append_text(Text.assemble(
        label("min gap "), value(f"{metadata.min_note_gap_ms:.1f}ms"),
        label("  same-key "), value(f"{metadata.min_same_key_gap_ms:.1f}ms"),
    ))
    warning = _warning_summary(metadata.warnings)
    if warning:
        txt.append("\n")
        txt.append_text(Text.assemble(
            label("warning "), value(warning, t.warning),
        ))
    return txt
