import sys
from unittest.mock import patch

from sky_music.infrastructure.realtime import RealtimeProcessScope, DISPATCH_SWITCH_INTERVAL_S


def test_realtime_scope_saves_and_restores_switch_interval() -> None:
    initial_interval = sys.getswitchinterval()
    # Ensure our target interval is different from initial to make the test meaningful
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        temp_interval = 0.005
        sys.setswitchinterval(temp_interval)
        initial_interval = temp_interval

    scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)
    with scope:
        assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_ablation_flag_respected() -> None:
    initial_interval = sys.getswitchinterval()
    # Ensure it's not the tuned value
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=False)
    with scope:
        assert abs(sys.getswitchinterval() - initial_interval) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_restores_on_exception() -> None:
    initial_interval = sys.getswitchinterval()
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    try:
        with RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True):
            assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7
            raise ValueError("Intentional exception")
    except ValueError:
        pass

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_nested_idempotence() -> None:
    initial_interval = sys.getswitchinterval()
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    scope1 = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)
    scope2 = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)

    with scope1:
        assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7
        with scope2:
            assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7
        assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_gc_pause_enabled() -> None:
    with patch("gc.isenabled", return_value=True) as mock_isenabled, \
         patch("gc.collect") as mock_collect, \
         patch("gc.disable") as mock_disable, \
         patch("gc.enable") as mock_enable:
        
        scope = RealtimeProcessScope(enabled=True, enable_switch_interval_tuning=False)
        with scope:
            mock_isenabled.assert_called_once()
            mock_collect.assert_called_once()
            mock_disable.assert_called_once()
            mock_enable.assert_not_called()

        mock_enable.assert_called_once()


def test_realtime_scope_gc_pause_disabled() -> None:
    with patch("gc.isenabled") as mock_isenabled, \
         patch("gc.collect") as mock_collect, \
         patch("gc.disable") as mock_disable, \
         patch("gc.enable") as mock_enable:
        
        scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=False)
        with scope:
            mock_isenabled.assert_not_called()
            mock_collect.assert_not_called()
            mock_disable.assert_not_called()

        mock_enable.assert_not_called()
