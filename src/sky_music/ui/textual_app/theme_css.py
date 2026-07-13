from __future__ import annotations

from sky_music.ui.picker_theme import THEME_PRESETS, ThemePreset

TextualThemeTokens = ThemePreset
TEXTUAL_THEME_TOKENS: dict[str, ThemePreset] = THEME_PRESETS


def _theme_css(name: str, t: TextualThemeTokens) -> str:
    """Generate the per-theme CSS block from design tokens."""
    s = f"Screen.theme-{name}"
    return f"""
    {s}.background-transparent {{ background: transparent; color: {t.foreground}; }}
    {s}.background-painted {{ background: {t.background}; color: {t.foreground}; }}
    {s} #appbar {{ background: transparent; }}
    {s} #search {{ background: transparent; border: round {t.border}; border-title-color: {t.muted}; }}
    {s} #search:focus {{ border: round {t.accent}; border-title-color: {t.accent}; }}
    {s} #songs {{
        background: transparent;
        border: round {t.accent};
        border-title-color: {t.accent};
        border-subtitle-color: {t.muted};
        scrollbar-size-vertical: 1;
        scrollbar-size-horizontal: 0;
        scrollbar-color: {t.accent};
        scrollbar-color-hover: {t.accent};
        scrollbar-color-active: {t.foreground};
        scrollbar-background: transparent;
        scrollbar-background-hover: transparent;
        scrollbar-background-active: transparent;
    }}
    {s} #detail {{ background: transparent; border: round {t.border}; border-title-color: {t.muted}; color: {t.detail}; }}
    {s} .datatable--header {{ background: transparent; color: {t.muted}; text-style: bold; }}
    {s} .datatable--cursor {{ background: {t.cursor_background}; color: {t.cursor_foreground}; text-style: bold; }}
    {s} AppFooter {{
        background: transparent;
    }}
    {s} ModalHintBar {{
        background: transparent;
    }}
    OptionModal.theme-{name} #modal-footer,
    InfoModal.theme-{name} #modal-footer {{
        border-top: none;
    }}
    CommandModal.theme-{name} #modal-footer {{
        border-top: none;
    }}
    OptionModal.theme-{name} OptionList > .option-list--option-disabled {{
        color: {t.key};
        background: transparent;
        text-style: bold;
    }}
    OptionModal.theme-{name} #modal,
    CommandModal.theme-{name} #modal,
    InfoModal.theme-{name} #modal {{
        background: {t.modal_background};
        border: round {t.border};
        border-title-color: {t.modal_title};
        border-title-style: bold;
        border-subtitle-color: {t.muted};
        border-subtitle-align: right;
    }}
    InfoModal.theme-{name} #info {{ color: {t.foreground}; }}
    OptionModal.theme-{name} #modal-info,
    CommandModal.theme-{name} #modal-info {{ color: {t.detail}; }}
    OptionModal.theme-{name} #modal-options,
    CommandModal.theme-{name} #modal-options {{
        background: transparent;
        border: none;
        color: {t.foreground};
    }}
    CommandModal.theme-{name} CommandPaletteList {{
        background: transparent;
        color: {t.foreground};
        scrollbar-color: {t.accent};
        scrollbar-color-hover: {t.accent};
        scrollbar-color-active: {t.foreground};
    }}
    CommandModal.theme-{name} CommandPaletteList > .option-list--option-highlighted {{
        background: {t.cursor_background};
        color: {t.cursor_foreground};
    }}
    CommandModal.theme-{name} #command-filter {{
        background: transparent;
        border: round {t.border};
        border-title-color: {t.muted};
        color: {t.foreground};
    }}
    CommandModal.theme-{name} #command-filter:focus {{
        border: round {t.accent};
        border-title-color: {t.accent};
    }}
    OptionModal.theme-{name} OptionList > .option-list--option-highlighted {{
        background: {t.cursor_background};
        color: {t.cursor_foreground};
    }}
    """


BASE_CSS = """
    Screen { background: transparent; }
    #root { height: 100%; layout: vertical; padding: 1 2; }
    #appbar { height: 3; }
    /* margin-bottom: 0 pulls search flush to the table so they read as one unit */
    #search { height: 3; margin: 1 0 0 0; padding: 0 1; }
    /* Extra horizontal padding so text doesn't press against the rounded border */
    #songs { height: 1fr; padding: 0 2; }
    /* max-height prevents detail panel from eating too much vertical space */
    #detail { height: auto; min-height: 5; max-height: 9; margin: 1 0 0 0; padding: 0 2; overflow-y: auto; }
    AppFooter { height: 1; margin: 1 0 0 0; }
    ModalHintBar { height: 1; margin-top: 1; content-align: center middle; }
    .datatable--cursor { text-style: bold; }
    /* Base modal layout - colours come from per-theme blocks */
    OptionModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    CommandModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    InfoModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    /* Modern modal: width derives from content (capped), height from content
       (capped to viewport). 76 cols keeps it readable on a 120-col terminal
       instead of stretching wide; max-width 90% keeps it inside thin terminals.
       Max-height 88% leaves room for the screen border + ambient padding. */
    #modal {
        width: 76;
        max-width: 90%;
        height: auto;
        max-height: 88%;
        padding: 1 2;
    }
    /* Modal body uses a vertical flex layout; children keep their natural
       heights and the palette grows to fill the remaining space. ``max-height``
       caps the modal so it never eats the entire viewport — we leave ~12% of
       the rows as ambient padding around the modal. ``overflow-y: auto`` on
       the body lets the *whole* modal body scroll when its natural content
       overflows the modal's own height (rare but possible on very short
       terminals where even the normal content does not fit). */
    #modal-content {
        height: 1fr;
        max-height: 100%;
        background: transparent;
        overflow-y: auto;
    }
    #modal-info {
        height: auto;
        max-height: 10;
        margin: 0 1 1 1;
        padding: 0 1;
    }
    /* Filter stays compact and flush — no left margin so it reads as part of
       the modal surface; bottom margin gives breathing room to the palette. */
    #command-filter {
        height: 3;
        margin: 0 1 1 1;
        padding: 0 1;
    }
    /* CommandModal-specific layout: filter (3) -> palette -> footer (1)
       inside #modal-content. Scroll behaviour is layered:
         * ``#modal`` (max-height 88%) caps the modal to ~88% of the viewport.
         * ``#modal-content`` (height 1fr + overflow-y auto) lets the body
           itself scroll if the natural content exceeds the modal alloc.
         * The palette's ``overflow-y: auto`` plus ``max-height: 18`` means
           once the option count (12 commands + 5 group headers = 17 rows)
           exceeds the allocated space, OptionList shows its own scrollbar
           and supports native wheel / PageDown / Home / End navigation.
       On tall terminals the palette stops growing at max-height 18 so the
       modal does not look sparse, and on short terminals the modal-content
       overflow-y takes over so the user can still scroll to reach the
       footer + filter even when the modal caps tightly. */
    CommandModal CommandPaletteList {
        height: auto;
        min-height: 4;
        max-height: 18;
        margin: 0 1;
        scrollbar-size-vertical: 1;
        scrollbar-background: transparent;
        overflow-y: auto;
    }
    #modal-options {
        height: auto;
        max-height: 24;
        background: transparent;
        overflow-y: auto;
        padding: 0 1;
    }
    #info {
        height: auto;
        max-height: 20;
        background: transparent;
        overflow-y: auto;
        padding: 0 1;
    }
    #modal-footer { height: 1; margin-top: 1; }
"""


APP_CSS = BASE_CSS + "\n".join(
    _theme_css(name, tokens) for name, tokens in TEXTUAL_THEME_TOKENS.items()
)
