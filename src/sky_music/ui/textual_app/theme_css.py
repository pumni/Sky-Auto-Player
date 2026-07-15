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
    InfoModal.theme-{name} #modal,
    UpdateSettingsModal.theme-{name} #modal {{
        background: {t.modal_background};
        border: round {t.border};
        border-title-color: {t.modal_title};
        border-title-style: bold;
        border-subtitle-color: {t.muted};
        border-subtitle-align: right;
    }}
    UpdateSettingsModal.theme-{name} #btn-check-now {{
        background: {t.accent_dim};
        color: {t.foreground};
        border: tall {t.accent_dim};
    }}
    UpdateSettingsModal.theme-{name} #btn-check-now:hover {{
        background: {t.cursor_background};
    }}
    UpdateSettingsModal.theme-{name} #btn-clear-skip {{
        background: {t.warning};
        color: {t.foreground};
        border: tall {t.warning};
    }}
    UpdateSettingsModal.theme-{name} #btn-clear-skip:hover {{
        background: {t.cursor_background};
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
    .datatable--cursor { text-style: bold; }
    AppFooter { dock: bottom; height: 1; margin: 1 0 0 0; }
    ModalHintBar { height: 1; margin-top: 1; content-align: center middle; }
    #modal-footer { height: 1; margin-top: 1; }
    /* Base modal layout - colours come from per-theme blocks */
    OptionModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    CommandModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    InfoModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    UpdateModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    UpdateSettingsModal { align: center middle; background: rgba(0, 0, 0, 0.6); }
    /* Wider modal; percentage cap keeps it from overflowing narrow terminals */
    #modal { width: 86; max-width: 92%; height: auto; max-height: 84%; padding: 1 2; }
    #modal-content { height: auto; max-height: 24; background: transparent; overflow-y: auto; }
    #modal-info { height: auto; max-height: 10; margin-bottom: 1; padding: 0 1; }
    #command-filter { height: 3; margin: 0 1 1 1; padding: 0 1; }
    #modal-options { height: auto; max-height: 24; background: transparent; overflow-y: auto; padding: 0 1; }
    #info { height: auto; max-height: 20; background: transparent; overflow-y: auto; }
    #update-info { height: auto; margin: 0 1 1 1; padding: 0 1; }
    #update-spacer { height: 1; }
    #update-caution { height: auto; margin: 1 0 0 0; padding: 0 1; }
    #update-progress-info { height: auto; margin: 0 1 1 1; padding: 0 1; }
    #update-progress-bar { margin: 0 1 1 1; }
    #update-progress-status { height: 1; margin: 0 1 1 1; padding: 0 1; }
    #update-notes { height: auto; max-height: 14; margin: 0 1 1 1; padding: 0 1; overflow-y: auto; }
    UpdateSettingsModal #update-settings-info {
        height: auto; max-height: 12; margin: 0 1 1 1; padding: 0 1;
    }
    UpdateSettingsModal #update-settings-divider { height: 1; margin: 0 1 1 1; }
    UpdateSettingsModal #update-settings-divider-2 { height: 1; margin: 1 1 1 1; }
    UpdateSettingsModal #update-settings-foot {
        height: auto; margin: 1 1 0 1; padding: 0 1;
    }
    /* Toggle rows — switch left, label right, vertically centered. */
    UpdateSettingsModal #row-auto-check,
    UpdateSettingsModal #row-auto-apply {
        height: auto; padding: 0 1;
        align-horizontal: left; align-vertical: top;
    }
    UpdateSettingsModal #row-auto-check Static,
    UpdateSettingsModal #row-auto-apply Static {
        margin-left: 1; width: 1fr;
    }
    UpdateSettingsModal #row-auto-check Checkbox, UpdateSettingsModal #row-auto-apply Checkbox { width: 3; height: auto; margin: 0; background: transparent; }
    /* Action buttons row — horizontal layout, wrap on narrow terminals. */
    UpdateSettingsModal #row-actions { height: auto; layout: horizontal; padding: 0 1; }
    UpdateSettingsModal #row-actions Button { margin: 0 1 0 0; }

    #playback-card {
        dock: bottom;
        width: 100%;
        padding: 0;
        background: transparent;
    }

    PlaybackApp Screen {
        align: center middle;
    }

    PlaybackApp #playback-card {
        width: 78;
        height: auto;
        padding: 1 2;
    }

    #song-name {
        text-align: center;
        text-style: bold;
        margin-bottom: 1;
    }

    #progress-bar {
        text-align: center;
        margin-bottom: 1;
    }

    #time-info {
        text-align: center;
        margin-bottom: 1;
    }

    #status-info {
        text-align: center;
        text-style: bold;
        margin-bottom: 1;
    }

    #warning-info {
        text-align: center;
        margin-bottom: 1;
    }

    #debug-panel {
        align: center middle;
        margin-top: 1;
        margin-bottom: 1;
        height: auto;
    }

    #debug-backend, #debug-lateness, #debug-timing {
        text-align: center;
    }

    #hotkeys-info {
        text-align: center;
    }
"""


APP_CSS = BASE_CSS + "\n".join(
    _theme_css(name, tokens) for name, tokens in TEXTUAL_THEME_TOKENS.items()
)
