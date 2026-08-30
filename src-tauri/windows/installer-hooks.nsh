; Upgrades keep every integration. A full uninstall asks the still-installed executable to
; remove only the status line and Stop hook that QuotaStation owns in Claude Code's settings.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-cleanup'
    Pop $0
    ${If} $0 != 0
      DetailPrint "QuotaStation could not remove one or more Claude Code integrations."
    ${EndIf}
  ${EndIf}
!macroend
