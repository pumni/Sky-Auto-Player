from unittest.mock import patch

import pytest

from sky_music.infrastructure.rt_priority import DispatchThreadPriorityScope


@pytest.fixture
def mock_win32_platform():
    """Mock sys.platform to be win32 so the priority ladder logic runs."""
    with patch("sky_music.infrastructure.rt_priority.sys.platform", "win32"):
        yield


@pytest.fixture
def mock_inputs():
    """Mock sky_music.platform.win32.inputs APIs."""
    with patch("sky_music.infrastructure.rt_priority.inputs") as mocked:
        # Defaults to make thread functions succeed normally
        mocked.get_current_thread.return_value = 123
        mocked.get_thread_priority.return_value = 0
        mocked.set_thread_priority.return_value = True
        mocked.av_set_mm_thread_characteristics.return_value = None
        mocked.disable_thread_power_throttling.return_value = False
        yield mocked


def test_priority_ladder_auto_success_mmcss(mock_win32_platform, mock_inputs) -> None:
    # MMCSS succeeds on the first try
    mock_inputs.av_set_mm_thread_characteristics.return_value = 9999
    
    scope = DispatchThreadPriorityScope("auto")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "mmcss:Pro Audio"
        assert scope.outcome.requested_mode == "auto"
        assert scope.outcome.detail is None
        mock_inputs.av_set_mm_thread_characteristics.assert_called_once_with("Pro Audio")
        mock_inputs.av_set_mm_thread_priority.assert_called_once_with(9999, 1)
        mock_inputs.set_thread_priority.assert_not_called()

    # Reverts on exit
    mock_inputs.av_revert_mm_thread_characteristics.assert_called_once_with(9999)


def test_priority_ladder_auto_fallback_to_time_critical(mock_win32_platform, mock_inputs) -> None:
    # MMCSS fails, time_critical succeeds
    mock_inputs.av_set_mm_thread_characteristics.return_value = None
    mock_inputs.get_current_thread.return_value = 123
    mock_inputs.get_thread_priority.return_value = 4  # original priority
    mock_inputs.set_thread_priority.return_value = True

    scope = DispatchThreadPriorityScope("auto")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "thread:time_critical"
        assert scope.outcome.detail is None
        
        # Verify it tried MMCSSNames
        assert mock_inputs.av_set_mm_thread_characteristics.call_count == 4
        # Verify it set thread priority to TIME_CRITICAL (15)
        mock_inputs.set_thread_priority.assert_called_once_with(123, 15)

    # Reverts thread priority to original on exit
    mock_inputs.set_thread_priority.assert_any_call(123, 4)
    assert mock_inputs.set_thread_priority.call_count == 2


def test_priority_ladder_auto_fallback_to_highest(mock_win32_platform, mock_inputs) -> None:
    # MMCSS fails, time_critical fails, highest succeeds
    mock_inputs.av_set_mm_thread_characteristics.return_value = None
    mock_inputs.get_current_thread.return_value = 123
    mock_inputs.get_thread_priority.return_value = 4

    def side_effect(thread_handle: int, priority: int) -> bool:
        if priority == 15:
            return False
        return priority == 2

    mock_inputs.set_thread_priority.side_effect = side_effect

    scope = DispatchThreadPriorityScope("auto")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "thread:highest"
        assert scope.outcome.detail is None

    # Reverts on exit
    mock_inputs.set_thread_priority.assert_any_call(123, 4)


def test_priority_ladder_auto_all_failed(mock_win32_platform, mock_inputs) -> None:
    mock_inputs.av_set_mm_thread_characteristics.return_value = None
    mock_inputs.set_thread_priority.return_value = False

    scope = DispatchThreadPriorityScope("auto")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "off"
        assert "Auto fall-through" in (scope.outcome.detail or "")


def test_priority_ladder_rung_fallback_on_exception(mock_win32_platform, mock_inputs) -> None:
    # MMCSS throws an exception, falls back to time_critical
    mock_inputs.av_set_mm_thread_characteristics.side_effect = RuntimeError("MMCSS Crash")
    mock_inputs.get_current_thread.return_value = 123
    mock_inputs.get_thread_priority.return_value = 4
    mock_inputs.set_thread_priority.return_value = True

    scope = DispatchThreadPriorityScope("auto")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "thread:time_critical"


def test_priority_ladder_explicit_mode_mmcss_success(mock_win32_platform, mock_inputs) -> None:
    mock_inputs.av_set_mm_thread_characteristics.return_value = 9999
    
    scope = DispatchThreadPriorityScope("mmcss")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "mmcss:Pro Audio"
        mock_inputs.set_thread_priority.assert_not_called()


def test_priority_ladder_explicit_mode_mmcss_failure(mock_win32_platform, mock_inputs) -> None:
    mock_inputs.av_set_mm_thread_characteristics.return_value = None
    
    scope = DispatchThreadPriorityScope("mmcss")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "off"
        assert "MMCSS failed" in (scope.outcome.detail or "")
        mock_inputs.set_thread_priority.assert_not_called()


def test_priority_ladder_explicit_mode_time_critical_success(mock_win32_platform, mock_inputs) -> None:
    mock_inputs.get_current_thread.return_value = 123
    mock_inputs.get_thread_priority.return_value = 4
    mock_inputs.set_thread_priority.return_value = True

    scope = DispatchThreadPriorityScope("time_critical")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "thread:time_critical"
        mock_inputs.av_set_mm_thread_characteristics.assert_not_called()


def test_priority_ladder_explicit_mode_time_critical_failure(mock_win32_platform, mock_inputs) -> None:
    mock_inputs.set_thread_priority.return_value = False

    scope = DispatchThreadPriorityScope("time_critical")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "off"
        assert "TIME_CRITICAL failed" in (scope.outcome.detail or "")
        # Should not fall back to highest
        mock_inputs.set_thread_priority.assert_called_once_with(mock_inputs.get_current_thread(), 15)


def test_priority_ladder_off_mode(mock_win32_platform, mock_inputs) -> None:
    scope = DispatchThreadPriorityScope("off")
    with scope:
        assert scope.outcome is not None
        assert scope.outcome.acquired == "off"
        assert scope.outcome.detail == "Disabled or non-win32 platform"
        
        # Verify no Win32 calls were made
        mock_inputs.av_set_mm_thread_characteristics.assert_not_called()
        mock_inputs.set_thread_priority.assert_not_called()


def test_priority_ladder_non_win32_platform(mock_inputs) -> None:
    # Ensure on non-win32 platform, it returns acquired='off' and calls nothing
    with patch("sky_music.infrastructure.rt_priority.sys.platform", "linux"):
        scope = DispatchThreadPriorityScope("auto")
        with scope:
            assert scope.outcome is not None
            assert scope.outcome.acquired == "off"
            mock_inputs.av_set_mm_thread_characteristics.assert_not_called()
            mock_inputs.set_thread_priority.assert_not_called()
