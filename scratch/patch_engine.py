import re

def main():
    path = r"D:\Dev\Sky-Auto-Player\rust\crates\sky_player_rs\src\engine.rs"
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
    
    # 1. PlaybackClockState::new
    content = re.sub(
        r"PlaybackClockState::new\(([^,]+),\s*0\)",
        r"PlaybackClockState::new(QpcTicks(qpc_us_to_ticks(\1)), sky_dispatch_core::time::DurationTicks(0))",
        content
    )

    # 2. rebase_epoch(now_us) -> qpc_ticks_to_us(QpcTicks(clock_state.rebase_epoch(QpcTicks(qpc_us_to_ticks(now_us))).0))
    # We must match clock_state.rebase_epoch(expr) and safely replace it.
    content = re.sub(
        r"clock_state\.rebase_epoch\(([^)]+)\)",
        r"qpc_ticks_to_us(QpcTicks(clock_state.rebase_epoch(QpcTicks(qpc_us_to_ticks(\1))).0))",
        content
    )
    
    # 3. get_elapsed_us(expr) -> qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(expr))).0))
    # Using a negative lookahead to make sure we don't accidentally match nested parens poorly,
    # though since there are no nested parens in get_elapsed_us calls in engine.rs, a simple regex is fine.
    # Let's list the exact expressions used in engine.rs to be extremely safe:
    # get_elapsed_us(now_us)
    # get_elapsed_us(started_us)
    # get_elapsed_us(result.send_completed_us)
    # get_elapsed_us(qpc_ticks_to_us(target_sample_ticks))
    # get_elapsed_us(qpc_now_us())
    content = re.sub(
        r"clock_state\.get_elapsed_us\(([^)]+)\)",
        r"qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(\1))).0))",
        content
    )
    # The nested parens in get_elapsed_us(qpc_ticks_to_us(target_sample_ticks)) and get_elapsed_us(qpc_now_us()) 
    # will break `([^)]+)`.
    # Let's use a function to match paired parentheses.
    def replace_elapsed(m):
        return f"qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks({m.group(1)}))).0))"
    
    # Reload original content to redo regex cleanly
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
        
    content = re.sub(
        r"PlaybackClockState::new\(([^,]+),\s*0\)",
        r"PlaybackClockState::new(QpcTicks(qpc_us_to_ticks(\1)), sky_dispatch_core::time::DurationTicks(0))",
        content
    )
    content = re.sub(
        r"clock_state\.rebase_epoch\(([^)]+)\)",
        r"qpc_ticks_to_us(QpcTicks(clock_state.rebase_epoch(QpcTicks(qpc_us_to_ticks(\1))).0))",
        content
    )
    content = content.replace(
        "clock_state.get_elapsed_us(now_us)",
        "qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(now_us))).0))"
    )
    content = content.replace(
        "clock_state.get_elapsed_us(started_us)",
        "qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(started_us))).0))"
    )
    content = content.replace(
        "clock_state.get_elapsed_us(result.send_completed_us)",
        "qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(result.send_completed_us))).0))"
    )
    content = content.replace(
        "clock_state.get_elapsed_us(qpc_ticks_to_us(target_sample_ticks))",
        "qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(qpc_ticks_to_us(target_sample_ticks)))).0))"
    )
    content = content.replace(
        "clock_state.get_elapsed_us(qpc_now_us())",
        "qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(qpc_now_us()))).0))"
    )
    
    # 4. enter_pause
    content = re.sub(
        r"clock_state\.enter_pause\(([^,]+),\s*([^)]+)\)",
        r"clock_state.enter_pause(\1, QpcTicks(qpc_us_to_ticks(\2)))",
        content
    )
    
    # 5. exit_pause
    content = re.sub(
        r"clock_state\.exit_pause\(([^,]+),\s*([^)]+)\)",
        r"clock_state.exit_pause(\1, QpcTicks(qpc_us_to_ticks(\2)))",
        content
    )

    # 6. epoch_us -> epoch
    content = content.replace(
        "clock_state.epoch_us",
        "qpc_ticks_to_us(clock_state.epoch)"
    )

    with open(path, "w", encoding="utf-8") as f:
        f.write(content)

if __name__ == "__main__":
    main()
