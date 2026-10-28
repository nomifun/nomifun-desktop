; Package-owned maintenance, never a command sent to an unknown old executable.
; The helper retains GUI and serial lifetime locks through apply/commit/abort.
Var NomiFunMaintenanceSession
Var NomiFunMaintenanceActive
Var NomiFunMaintenanceStarted
Var NomiFunMaintenanceFinished
Var NomiFunMaintenanceExpected
Var NomiFunMaintenanceState
Var NomiFunMaintenanceDetail
Var NomiFunMaintenanceRemaining
Var NomiFunMaintenanceResult
Var NomiFunMaintenanceCommand

!macro NomiFunMaintenanceFunctions PREFIX UNINSTALL
  !ifndef NOMIFUN_MAINTENANCE_BINARY
    !searchreplace NOMIFUN_MAINTENANCE_BINARY "${MAINBINARYSRCPATH}" "${MAINBINARYNAME}.exe" "nomifun-keyboard-maintenance.exe"
  !endif

  Function ${PREFIX}NomiFunMaintenancePublish
    ClearErrors
    FileOpen $NomiFunMaintenanceResult "$NomiFunMaintenanceSession\command.next" w
    FileWrite $NomiFunMaintenanceResult $NomiFunMaintenanceCommand
    FileClose $NomiFunMaintenanceResult
    ${If} ${Errors}
      StrCpy $NomiFunMaintenanceDetail "Could not write the maintenance request."
      StrCpy $NomiFunMaintenanceResult 0
      Return
    ${EndIf}
    ; Abort must be able to supersede an unconsumed apply/commit request. NSIS
    ; Rename does not replace an existing destination on Windows.
    Push $0
    System::Call 'kernel32::MoveFileExW(w "$NomiFunMaintenanceSession\command.next", w "$NomiFunMaintenanceSession\command", i 9)i.r0'
    StrCpy $NomiFunMaintenanceResult $0
    Pop $0
    ${If} $NomiFunMaintenanceResult == 0
      StrCpy $NomiFunMaintenanceDetail "Could not publish the maintenance request."
    ${EndIf}
  FunctionEnd

  Function ${PREFIX}NomiFunMaintenanceWait
    StrCpy $NomiFunMaintenanceResult 0
    StrCpy $NomiFunMaintenanceRemaining 1800
    ${Do}
      ReadINIStr $NomiFunMaintenanceState "$NomiFunMaintenanceSession\status.ini" "maintenance" "state"
      ReadINIStr $NomiFunMaintenanceDetail "$NomiFunMaintenanceSession\status.ini" "maintenance" "detail"
      ${If} $NomiFunMaintenanceState == $NomiFunMaintenanceExpected
        StrCpy $NomiFunMaintenanceResult 1
        ${ExitDo}
      ${EndIf}
      ${If} $NomiFunMaintenanceExpected == "aborted"
      ${AndIf} $NomiFunMaintenanceState == "committed"
        ; A commit already claimed by the helper is final. Closing the window
        ; must not wait for a rollback that can no longer take place.
        StrCpy $NomiFunMaintenanceResult 1
        ${ExitDo}
      ${EndIf}
      ${If} $NomiFunMaintenanceState == "error"
      ${OrIf} $NomiFunMaintenanceState == "repair_required"
        ${ExitDo}
      ${EndIf}
      IntOp $NomiFunMaintenanceRemaining $NomiFunMaintenanceRemaining - 1
      ${If} $NomiFunMaintenanceRemaining <= 0
        StrCpy $NomiFunMaintenanceDetail "The maintenance helper did not finish in time. The installed application has not been confirmed ready."
        ${ExitDo}
      ${EndIf}
      Sleep 50
    ${Loop}
  FunctionEnd

  Function ${PREFIX}NomiFunMaintenanceStart
    InitPluginsDir
    StrCpy $NomiFunMaintenanceSession "$PLUGINSDIR\keyboard-maintenance"
    CreateDirectory "$NomiFunMaintenanceSession"
    SetOutPath $PLUGINSDIR
    ClearErrors
    File /oname=nomifun-keyboard-maintenance.exe "${NOMIFUN_MAINTENANCE_BINARY}"
    ${If} ${Errors}
      MessageBox MB_OK|MB_ICONSTOP "NomiFun could not extract its installation maintenance helper." /SD IDOK
      Abort
    ${EndIf}
    StrCpy $NomiFunMaintenanceDetail ""
    ${If} $UpdateMode == 1
      StrCpy $NomiFunMaintenanceDetail "--resume-app --defer-app-on-commit"
    ${EndIf}
    Push $0
    System::Call 'kernel32::GetCurrentProcessId()i.r0'
    !if "${UNINSTALL}" == "yes"
      Exec '"$PLUGINSDIR\nomifun-keyboard-maintenance.exe" --session "$NomiFunMaintenanceSession" --installed "$INSTDIR\${MAINBINARYNAME}.exe" --parent-pid $0 --uninstall'
    !else
      Exec '"$PLUGINSDIR\nomifun-keyboard-maintenance.exe" --session "$NomiFunMaintenanceSession" --installed "$INSTDIR\${MAINBINARYNAME}.exe" --parent-pid $0 --payload "$PLUGINSDIR\payload" $NomiFunMaintenanceDetail'
    !endif
    Pop $0
    ${If} ${Errors}
      MessageBox MB_OK|MB_ICONSTOP "NomiFun could not start its installation maintenance helper." /SD IDOK
      Abort
    ${EndIf}
    StrCpy $NomiFunMaintenanceActive 1
    StrCpy $NomiFunMaintenanceStarted 1
    StrCpy $NomiFunMaintenanceExpected "ready"
    Call ${PREFIX}NomiFunMaintenanceWait
    ${If} $NomiFunMaintenanceResult != 1
      DetailPrint "Keyboard maintenance: $NomiFunMaintenanceDetail"
      MessageBox MB_OK|MB_ICONSTOP "NomiFun could not prepare the application for installation.$\r$\n$\r$\n$NomiFunMaintenanceDetail" /SD IDOK
      Abort
    ${EndIf}
    SetOutPath $INSTDIR
  FunctionEnd

  Function ${PREFIX}NomiFunMaintenanceApply
    StrCpy $NomiFunMaintenanceCommand "apply"
    Call ${PREFIX}NomiFunMaintenancePublish
    ${If} $NomiFunMaintenanceResult == 0
      Abort "$NomiFunMaintenanceDetail"
    ${EndIf}
    StrCpy $NomiFunMaintenanceExpected "applied"
    Call ${PREFIX}NomiFunMaintenanceWait
    ${If} $NomiFunMaintenanceResult != 1
      DetailPrint "Keyboard maintenance: $NomiFunMaintenanceDetail"
      MessageBox MB_OK|MB_ICONSTOP "NomiFun could not replace the application files.$\r$\n$\r$\n$NomiFunMaintenanceDetail" /SD IDOK
      Abort
    ${EndIf}
  FunctionEnd

  Function ${PREFIX}NomiFunMaintenanceCommit
    ${If} $NomiFunMaintenanceActive == 1
      StrCpy $NomiFunMaintenanceCommand "commit"
      Call ${PREFIX}NomiFunMaintenancePublish
      ${If} $NomiFunMaintenanceResult == 0
        Abort "$NomiFunMaintenanceDetail"
      ${EndIf}
      StrCpy $NomiFunMaintenanceExpected "committed"
      Call ${PREFIX}NomiFunMaintenanceWait
      ${If} $NomiFunMaintenanceResult != 1
        MessageBox MB_OK|MB_ICONSTOP "NomiFun could not complete installation maintenance.$\r$\n$\r$\n$NomiFunMaintenanceDetail" /SD IDOK
        Abort
      ${EndIf}
      StrCpy $NomiFunMaintenanceActive 0
      StrCpy $NomiFunMaintenanceFinished 1
      ${If} $NomiFunMaintenanceDetail != ""
        DetailPrint "Application restart: $NomiFunMaintenanceDetail"
        MessageBox MB_OK|MB_ICONEXCLAMATION "NomiFun was installed, but its application restart needs attention.$\r$\n$\r$\n$NomiFunMaintenanceDetail" /SD IDOK
      ${EndIf}
    ${EndIf}
  FunctionEnd

  Function ${PREFIX}NomiFunMaintenanceAbort
    ${If} $NomiFunMaintenanceActive == 1
      ReadINIStr $NomiFunMaintenanceState "$NomiFunMaintenanceSession\status.ini" "maintenance" "state"
      ${If} $NomiFunMaintenanceState == "committed"
      ${OrIf} $NomiFunMaintenanceState == "aborted"
        StrCpy $NomiFunMaintenanceActive 0
        StrCpy $NomiFunMaintenanceFinished 1
        Return
      ${EndIf}
      ${If} $NomiFunMaintenanceState != "error"
      ${AndIf} $NomiFunMaintenanceState != "repair_required"
        StrCpy $NomiFunMaintenanceCommand "abort"
        Call ${PREFIX}NomiFunMaintenancePublish
        ${If} $NomiFunMaintenanceResult != 0
          StrCpy $NomiFunMaintenanceExpected "aborted"
          Call ${PREFIX}NomiFunMaintenanceWait
        ${EndIf}
      ${EndIf}
      ${If} $NomiFunMaintenanceResult != 1
      ${AndIf} $NomiFunMaintenanceState != "error"
        MessageBox MB_OK|MB_ICONSTOP "NomiFun could not confirm recovery.$\r$\n$\r$\n$NomiFunMaintenanceDetail" /SD IDOK
      ${EndIf}
      StrCpy $NomiFunMaintenanceActive 0
      StrCpy $NomiFunMaintenanceFinished 1
    ${EndIf}
    !if "${UNINSTALL}" == "no"
      ; The updater has already exited its GUI before NSIS extracts payload or
      ; begins maintenance. Cancellation/extraction failure there still owes
      ; the user the unchanged installed application, even without a helper.
      ${If} $UpdateMode == 1
      ${AndIf} $NomiFunMaintenanceStarted != 1
      ${AndIf} $NomiFunMaintenanceFinished != 1
        StrCpy $NomiFunMaintenanceFinished 1
        ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
          Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
        ${EndIf}
      ${EndIf}
    !endif
  FunctionEnd
!macroend

!macro NSIS_HOOK_PREINSTALL
  Call NomiFunMaintenanceStart
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Call un.NomiFunMaintenanceStart
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Call un.NomiFunMaintenanceCommit
!macroend
