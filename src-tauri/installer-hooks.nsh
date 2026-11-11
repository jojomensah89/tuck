; Tuck shows its welcome page once after the installer runs, but not after an
; automatic update (the updater passes /UPDATE). The app deletes the marker
; when it shows the page.
!macro NSIS_HOOK_POSTINSTALL
  ${If} $UpdateMode <> 1
    CreateDirectory "$APPDATA\com.jojo.tuck"
    FileOpen $0 "$APPDATA\com.jojo.tuck\show-welcome" w
    FileClose $0
  ${EndIf}
!macroend
