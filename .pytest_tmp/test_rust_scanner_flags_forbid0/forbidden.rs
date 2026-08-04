let dll = "ntdll.dll";
unsafe { SetWindowsHookExW(0, hook, 0, 0); }
