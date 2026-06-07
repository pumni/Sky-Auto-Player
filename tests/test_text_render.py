from __future__ import annotations

from sky_music.ui.text_render import ansi_box, ansi_gradient_box, visible_width


def test_ansi_box_truncates_long_colored_lines_to_keep_border_width() -> None:
    width = 32
    long_line = "\033[33m" + ("very long warning " * 8) + "\033[0m"

    rendered = ansi_box("HUD", [long_line], width=width, border_color="\033[36m")

    assert all(visible_width(line) == width for line in rendered)
    assert "…" in rendered[1]


def test_ansi_gradient_box_truncates_long_colored_lines_to_keep_border_width() -> None:
    width = 36
    long_line = "\033[31m" + ("backend status " * 8) + "\033[0m"

    rendered = ansi_gradient_box(
        "HUD",
        [long_line],
        width=width,
        gradient=("#38bdf8", "#a78bfa"),
        title_color="#ffffff",
    )

    assert all(visible_width(line) == width for line in rendered)
    assert "…" in rendered[1]
