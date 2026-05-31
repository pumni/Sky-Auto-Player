from typing import Any, Literal
from dataclasses import dataclass
from sky_music.ui.picker_theme import truncate_text, get_match_span

@dataclass(frozen=True, slots=True)
class ActionHint:
    key: str
    long: str
    short: str
    tiny: str

def format_actions(actions: list[ActionHint], width: int) -> list[tuple[str, str]]:
    # Try long first
    tokens = []
    for act in actions:
        tokens.append(("class:key", act.key))
        tokens.append(("class:footer", f" {act.long} │ "))
    if tokens:
        tokens[-1] = ("class:footer", f" {actions[-1].long}")
    total_len = sum(len(text) for _, text in tokens)
    if total_len <= width:
        return tokens
        
    # Try short
    tokens = []
    for act in actions:
        tokens.append(("class:key", act.key))
        tokens.append(("class:footer", f" {act.short} │ "))
    if tokens:
        tokens[-1] = ("class:footer", f" {actions[-1].short}")
    total_len = sum(len(text) for _, text in tokens)
    if total_len <= width:
        return tokens
        
    # Try tiny
    tokens = []
    for act in actions:
        tokens.append(("class:key", act.key))
        tokens.append(("class:footer", f" {act.tiny} │ "))
    if tokens:
        tokens[-1] = ("class:footer", f" {actions[-1].tiny}")
    return tokens

def _format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, sec = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}:{minutes:02}:{sec:02}"
    return f"{minutes}:{sec:02}"

def build_box(title: str, content: list[Any], width: int = 76) -> list[tuple[str, str]]:
    top_left = "╭"
    top_right = "╮"
    bottom_left = "╰"
    bottom_right = "╯"
    horiz = "─"
    vert = "│"
    
    title_part = f"{horiz} {title} "
    top_line = f"{top_left}{title_part}{horiz * (width - len(title_part) - 2)}{top_right}\n"
    bottom_line = f"{bottom_left}{horiz * (width - 2)}{bottom_right}\n"
    
    tokens = [("class:divider", top_line)]
    for line in content:
        tokens.append(("class:divider", f"{vert} "))
        if isinstance(line, str):
            line_clean = line[:width - 4]
            tokens.append(("class:detail", f"{line_clean:<{width - 4}}"))
        else:
            line_len = 0
            for style, text in line:
                if line_len + len(text) > width - 4:
                    text = text[:width - 4 - line_len]
                tokens.append((style, text))
                line_len += len(text)
            if line_len < width - 4:
                tokens.append(("class:detail", " " * (width - 4 - line_len)))
        tokens.append(("class:divider", f" {vert}\n"))
    tokens.append(("class:divider", bottom_line))
    return tokens

def format_song_row(idx: int, metadata: Any, selected: bool, query: str, pointer: str, song_icon: str) -> list[tuple[str, str]]:
    dur_str = _format_duration(metadata.duration_seconds)
    risk_upper = metadata.risk.upper()[:5]
    
    risk_style = "fg:#f97316 bold" if risk_upper == "HIGH" else ("fg:#fbbf24 bold" if risk_upper == "MED" or risk_upper == "MEDIUM" else "fg:#10b981")
    if selected:
        risk_style = "fg:#ffffff bold"
        
    tokens = []
    prefix = f"{pointer} {idx:<3} " if selected else f"  {idx:<3} "
    sel_class = "class:selected" if selected else "class:unselected"
    
    tokens.append((sel_class, prefix))
    
    song_name = metadata.name
    song_name_trunc = truncate_text(song_name, 36)
    
    if selected:
        tokens.append(("class:selected", f"{song_name_trunc:<36}"))
    else:
        span = get_match_span(song_name_trunc, query)
        if span is None:
            tokens.append(("class:unselected", f"{song_name_trunc:<36}"))
        else:
            start, end = span
            tokens.append(("class:unselected", song_name_trunc[:start]))
            tokens.append(("class:match", song_name_trunc[start:end]))
            tokens.append(("class:unselected", f"{song_name_trunc[end:]:<{36 - len(song_name_trunc)}}"))
            
    tokens.append((sel_class, f"    {dur_str:>4}   {metadata.note_count:>5}   "))
    tokens.append((risk_style if not selected else sel_class, f"{risk_upper:<5}"))
    tokens.append((sel_class, f"   {metadata.recommended_profile.strip():<11}\n"))
    return tokens

def format_info_str(parts: list[str], max_width: int) -> str:
    current_parts = list(parts)
    while current_parts:
        candidate = " │ ".join(current_parts)
        if len(candidate) <= max_width:
            return candidate
        current_parts.pop()
    return parts[0]
