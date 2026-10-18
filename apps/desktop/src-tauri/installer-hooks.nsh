; NSIS hooks of the blurrd installer (bundle > windows > nsis >
; installerHooks in tauri.conf.json). Tauri includes this file at the top
; of its installer script.
;
; The per-user installer would install into %LOCALAPPDATA%\blurrd, the
; folder where blurrd keeps its cached projects (docs/privacy.md says it
; can be deleted at any time). blurrd installs into
; %LOCALAPPDATA%\Programs\blurrd instead, where Windows keeps programs
; installed for one user.

!define BLURRD_DATA_DIR "$LOCALAPPDATA\blurrd"
!define BLURRD_PROGRAM_DIR "$LOCALAPPDATA\Programs\blurrd"

; The directory page shows the program folder as the default.
!define MUI_CUSTOMFUNCTION_GUIINIT BlurrdDefaultDir
Function BlurrdDefaultDir
  ${If} $INSTDIR == "${BLURRD_DATA_DIR}"
    StrCpy $INSTDIR "${BLURRD_PROGRAM_DIR}"
  ${EndIf}
FunctionEnd

; Silent installs never show that page, and the data folder is never a
; valid choice: install into the program folder whenever it was picked.
!macro NSIS_HOOK_PREINSTALL
  ${If} $INSTDIR == "${BLURRD_DATA_DIR}"
    StrCpy $INSTDIR "${BLURRD_PROGRAM_DIR}"
    SetOutPath $INSTDIR
  ${EndIf}
!macroend

; Tauri's uninstaller keeps the install location under
; HKCU\Software\<publisher>\blurrd unless "Delete the application data" is
; ticked, and that option leaves blurrd's cached projects (which hold
; unblurred preview frames) behind. Remove the key on every uninstall,
; and the cache together with the other application data; updates keep
; both.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
    ${If} $DeleteAppDataCheckboxState = 1
      RmDir /r "${BLURRD_DATA_DIR}"
    ${EndIf}
  ${EndIf}
!macroend
