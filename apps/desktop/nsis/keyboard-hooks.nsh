; Native keyboard service lifecycle. The authenticated management request
; quiesces the desktop reconnect loop before the serial owner exits.
!macro NomiFunStopKeyboard MODE
  ; A GUI microphone test can run without a serial owner. Always ask the
  ; installed CLI to quiesce both authenticated endpoints before replacement.
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    Push $0
    Push $1
    nsExec::ExecToStack /TIMEOUT=90000 '"$INSTDIR\${MAINBINARYNAME}.exe" ${MODE}'
    Pop $0
    Pop $1
    ${If} $0 != 0
      MessageBox MB_OK|MB_ICONSTOP "NomiFun could not confirm that its keyboard service stopped. Installation was stopped before replacing any files. Quit all NomiFun services and retry."
      Abort
    ${EndIf}
    Pop $1
    Pop $0
  ${Else}
    ${If} ${FileExists} "$LOCALAPPDATA\NomiFun\keyboard-service\owner.json"
    ${OrIf} ${FileExists} "$LOCALAPPDATA\NomiFun\keyboard-service\desktop.json"
      MessageBox MB_OK|MB_ICONSTOP "NomiFun has an active endpoint descriptor, but its installed executable is missing. Restore the installed executable and quit all NomiFun services before continuing."
      Abort
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro NomiFunStopKeyboard "--keyboard-stop"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode = 1
    !insertmacro NomiFunStopKeyboard "--keyboard-stop"
  ${Else}
    !insertmacro NomiFunStopKeyboard "--keyboard-unregister"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "NomiFunKeyboard"
  ${EndIf}
!macroend
