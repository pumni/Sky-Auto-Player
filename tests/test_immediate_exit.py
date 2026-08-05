"""Tests for immediate exit lifecycle safety during catalog scan."""

import time
from unittest.mock import patch

from sky_music.ui.textual_app.screens.picker import PickerScreen


def test_picker_immediate_exit_during_catalog_scan() -> None:
    """Ensure that exiting the app while catalog scan is running does not deadlock or error."""
    import asyncio

    # We create a dummy app and mount PickerScreen
    from sky_music.ui.textual_app.app import SkyPickerApp
    
    app = SkyPickerApp()
    
    # We will patch get_song_choices to block for a bit, simulating slow I/O
    def mock_get_song_choices(*args, **kwargs):
        time.sleep(0.5)
        return []

    async def run_it():
        with patch("sky_music.ui.picker_helpers.get_song_choices", side_effect=mock_get_song_choices):
            async with app.run_test() as pilot:
                # wait for screen to mount and deferred startup to fire
                await pilot.pause(0.1)
                
                # get the picker screen
                picker = app.screen
                assert isinstance(picker, PickerScreen)
                
                # verify that catalog executor is running
                # now we exit the app while it's blocked
                await app.action_quit()
                
    asyncio.run(run_it())
