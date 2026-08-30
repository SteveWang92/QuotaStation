; A full uninstall asks the still-installed executable to remove only the status line and
; Stop hook that QuotaStation owns in Claude Code's settings. NSIS runs the old uninstaller
; the same way when a newly downloaded installer replaces an existing copy, so this runs on
; that path too; `restore-integrations.json` is what puts the integrations back on the next
; start. Only the updater's `/UPDATE` uninstall keeps them untouched.
;
; The running-instance check belongs here rather than where the template performs it: the
; uninstall is almost always started while the tray application is running, and cancelling
; that prompt aborts the uninstall after this hook has already run.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-cleanup'
    Pop $0
    ${If} $0 != 0
      DetailPrint "QuotaStation could not remove one or more Claude Code integrations."
    ${EndIf}
  ${EndIf}
!macroend
