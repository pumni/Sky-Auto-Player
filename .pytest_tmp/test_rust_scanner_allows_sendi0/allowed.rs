// keybd_event must remain forbidden
/* SetWindowsHookExW is forbidden too. */
unsafe { SendInput(1, inputs, size); }
