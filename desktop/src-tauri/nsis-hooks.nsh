; v4.1.4 publisher migration hook.
; The stable uninstall identity owns the install root. Do not let a publisher
; namespace change select a stale or default root during an update.
!macro NSIS_HOOK_PREINSTALL
  ; The updater supplies /D with the current executable's root. Prefer it when
  ; it names an existing installation; this also survives a child installer
  ; process that cannot see the caller's HKCU registry view.
  IfFileExists "$INSTDIR\sky_desktop_shell.exe" sap_migration_explicit_root sap_migration_read_registry
  sap_migration_explicit_root:
    Goto sap_migration_root_checked
  sap_migration_read_registry:
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Sky Auto Player" "InstallLocation"
  ${If} $R0 != ""
    StrCpy $R1 $R0 1
    StrCmp $R1 $\" sap_migration_strip_quotes sap_migration_root_checked
    sap_migration_strip_quotes:
      StrLen $R2 $R0
      IntOp $R2 $R2 - 2
      StrCpy $R0 $R0 $R2 1
    sap_migration_root_checked:
    StrCpy $INSTDIR $R0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Remove the historical publisher namespace after the canonical uninstall
  ; identity has been rewritten with the candidate publisher.
  DeleteRegKey SHELL_CONTEXT "Software\github\Sky Auto Player"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Publisher registry keys are installer state, not user application data.
  ; Remove both namespaces so silent uninstall cannot leave stale lineage.
  DeleteRegKey SHELL_CONTEXT "Software\github\Sky Auto Player"
  DeleteRegKey SHELL_CONTEXT "Software\pumni\Sky Auto Player"
!macroend
