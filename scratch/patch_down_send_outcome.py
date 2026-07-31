import re

def main():
    path = r"D:\Dev\Sky-Auto-Player\rust\crates\sky_player_rs\src\engine.rs"
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()

    # The code we want to replace starts right after `let result = backend.key_down(&scan_codes);`
    # We will insert the match statement right after it.
    
    match_code = """
                        let (
                            result_completed_us,
                            result_sent,
                            result_skipped_duplicates,
                            result_send_attempts,
                            result_zero_progress_retries,
                            result_retried_after_zero_progress,
                            result_chord_integrity_lost,
                            result_first_win32_error,
                            result_last_win32_error,
                            result_success,
                        ) = match &result {
                            sky_dispatch_win32::input::DownSendOutcome::Complete {
                                completed_us, sent, skipped_duplicates, send_attempts, zero_progress_retries, retried_after_zero_progress
                            } => (
                                *completed_us, sent.clone(), skipped_duplicates.clone(), *send_attempts, *zero_progress_retries, *retried_after_zero_progress, false, None, None, true
                            ),
                            sky_dispatch_win32::input::DownSendOutcome::ZeroProgress {
                                completed_us, skipped_duplicates, send_attempts, zero_progress_retries, first_error, last_error, ..
                            } => (
                                *completed_us, smallvec::SmallVec::<[u16; 15]>::new(), skipped_duplicates.clone(), *send_attempts, *zero_progress_retries, *zero_progress_retries > 0, false, *first_error, *last_error, false
                            ),
                            sky_dispatch_win32::input::DownSendOutcome::IntegrityLost {
                                completed_us, sent, skipped_duplicates, send_attempts, zero_progress_retries, first_error, last_error, ..
                            } => (
                                *completed_us, sent.clone(), skipped_duplicates.clone(), *send_attempts, *zero_progress_retries, *zero_progress_retries > 0, true, *first_error, *last_error, false
                            ),
                        };
"""
    # Replace usages of `result.xxx` with `result_xxx` in the down block
    
    # We only want to replace within the `if send_playable && !playable.is_empty() {` block.
    # It starts at: `let result = backend.key_down(&scan_codes);`
    
    parts = content.split("let result = backend.key_down(&scan_codes);")
    if len(parts) == 2:
        down_block = parts[1]
        
        # Replace occurrences
        down_block = down_block.replace("result.send_completed_us", "result_completed_us")
        down_block = down_block.replace("result.sent", "result_sent")
        down_block = down_block.replace("result.skipped_duplicates", "result_skipped_duplicates")
        down_block = down_block.replace("result.send_attempts", "result_send_attempts")
        down_block = down_block.replace("result.zero_progress_retries", "result_zero_progress_retries")
        down_block = down_block.replace("result.retried_after_zero_progress", "result_retried_after_zero_progress")
        down_block = down_block.replace("result.chord_integrity_lost", "result_chord_integrity_lost")
        down_block = down_block.replace("result.first_win32_error", "result_first_win32_error")
        down_block = down_block.replace("result.last_win32_error", "result_last_win32_error")
        down_block = down_block.replace("result.success", "result_success")
        
        # There is one `result.chord_integrity_lost` check which is inside `if result.chord_integrity_lost { force_full_cleanup = true; }`
        
        new_content = parts[0] + "let result = backend.key_down(&scan_codes);\n" + match_code + down_block
        
        with open(path, "w", encoding="utf-8") as f:
            f.write(new_content)
    else:
        print("Failed to find exactly one occurrence of `let result = backend.key_down(&scan_codes);`")

if __name__ == "__main__":
    main()
