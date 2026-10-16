; File types on Windows, registered the way Windows asks apps to: ReVLC is
; offered for them (Open with, the "Select an app" prompt, Settings > Default
; apps) but never made the default. Tauri's own associations wrote each
; extension's default handler, which took files from VLC and others without
; asking; tauri.windows.conf.json turns those off.

!define REVLC_CAPS "Software\ReVLC\Capabilities"
!define REVLC_EXE "$INSTDIR\${MAINBINARYNAME}.exe"

; Calls MACRO with each extension and its ProgID.
!macro REVLC_FOR_EACH_TYPE MACRO
  !insertmacro ${MACRO} "mp4" "ReVLC.Video"
  !insertmacro ${MACRO} "m4v" "ReVLC.Video"
  !insertmacro ${MACRO} "mkv" "ReVLC.Video"
  !insertmacro ${MACRO} "webm" "ReVLC.Video"
  !insertmacro ${MACRO} "mov" "ReVLC.Video"
  !insertmacro ${MACRO} "avi" "ReVLC.Video"
  !insertmacro ${MACRO} "ts" "ReVLC.Video"
  !insertmacro ${MACRO} "m2ts" "ReVLC.Video"
  !insertmacro ${MACRO} "mpg" "ReVLC.Video"
  !insertmacro ${MACRO} "mpeg" "ReVLC.Video"
  !insertmacro ${MACRO} "wmv" "ReVLC.Video"
  !insertmacro ${MACRO} "flv" "ReVLC.Video"
  !insertmacro ${MACRO} "ogv" "ReVLC.Video"
  !insertmacro ${MACRO} "3gp" "ReVLC.Video"
  !insertmacro ${MACRO} "mts" "ReVLC.Video"
  !insertmacro ${MACRO} "m2t" "ReVLC.Video"
  !insertmacro ${MACRO} "vob" "ReVLC.Video"
  !insertmacro ${MACRO} "divx" "ReVLC.Video"
  !insertmacro ${MACRO} "rm" "ReVLC.Video"
  !insertmacro ${MACRO} "rmvb" "ReVLC.Video"
  !insertmacro ${MACRO} "asf" "ReVLC.Video"
  !insertmacro ${MACRO} "f4v" "ReVLC.Video"
  !insertmacro ${MACRO} "3g2" "ReVLC.Video"
  !insertmacro ${MACRO} "ogm" "ReVLC.Video"
  !insertmacro ${MACRO} "mxf" "ReVLC.Video"
  !insertmacro ${MACRO} "mp3" "ReVLC.Audio"
  !insertmacro ${MACRO} "m4a" "ReVLC.Audio"
  !insertmacro ${MACRO} "aac" "ReVLC.Audio"
  !insertmacro ${MACRO} "flac" "ReVLC.Audio"
  !insertmacro ${MACRO} "wav" "ReVLC.Audio"
  !insertmacro ${MACRO} "ogg" "ReVLC.Audio"
  !insertmacro ${MACRO} "opus" "ReVLC.Audio"
  !insertmacro ${MACRO} "wma" "ReVLC.Audio"
  !insertmacro ${MACRO} "aiff" "ReVLC.Audio"
  !insertmacro ${MACRO} "mka" "ReVLC.Audio"
  !insertmacro ${MACRO} "ape" "ReVLC.Audio"
  !insertmacro ${MACRO} "wv" "ReVLC.Audio"
  !insertmacro ${MACRO} "tta" "ReVLC.Audio"
  !insertmacro ${MACRO} "mpc" "ReVLC.Audio"
  !insertmacro ${MACRO} "ac3" "ReVLC.Audio"
  !insertmacro ${MACRO} "dts" "ReVLC.Audio"
  !insertmacro ${MACRO} "oga" "ReVLC.Audio"
  !insertmacro ${MACRO} "spx" "ReVLC.Audio"
  !insertmacro ${MACRO} "m4b" "ReVLC.Audio"
  !insertmacro ${MACRO} "amr" "ReVLC.Audio"
  !insertmacro ${MACRO} "aif" "ReVLC.Audio"
  !insertmacro ${MACRO} "aifc" "ReVLC.Audio"
  !insertmacro ${MACRO} "m3u" "ReVLC.Playlist"
  !insertmacro ${MACRO} "m3u8" "ReVLC.Playlist"
  !insertmacro ${MACRO} "pls" "ReVLC.Playlist"
  !insertmacro ${MACRO} "xspf" "ReVLC.Playlist"
  !insertmacro ${MACRO} "asx" "ReVLC.Playlist"
  ; Registered by 0.2.x but not a real extension; listed so cleanup finds it.
  !insertmacro ${MACRO} "alac" ""
!macroend

; 0.2.x set ".ext" to a ProgID called Video, Audio or Playlist and kept the
; old value in "<ProgID>_backup". Put back whatever was there before ReVLC.
!macro REVLC_UNDO_OLD EXT PROGID
  ReadRegStr $R0 SHCTX "Software\Classes\.${EXT}" ""
  ${If} $R0 == "Video"
  ${OrIf} $R0 == "Audio"
  ${OrIf} $R0 == "Playlist"
    ${If} $R0 == "Video"
      ReadRegStr $R1 SHCTX "Software\Classes\.${EXT}" "Video_backup"
    ${ElseIf} $R0 == "Audio"
      ReadRegStr $R1 SHCTX "Software\Classes\.${EXT}" "Audio_backup"
    ${Else}
      ReadRegStr $R1 SHCTX "Software\Classes\.${EXT}" "Playlist_backup"
    ${EndIf}
    ${If} $R1 == ""
    ${OrIf} $R1 == "Video"
    ${OrIf} $R1 == "Audio"
    ${OrIf} $R1 == "Playlist"
      DeleteRegValue SHCTX "Software\Classes\.${EXT}" ""
    ${Else}
      WriteRegStr SHCTX "Software\Classes\.${EXT}" "" "$R1"
    ${EndIf}
  ${EndIf}
  DeleteRegValue SHCTX "Software\Classes\.${EXT}" "Video_backup"
  DeleteRegValue SHCTX "Software\Classes\.${EXT}" "Audio_backup"
  DeleteRegValue SHCTX "Software\Classes\.${EXT}" "Playlist_backup"
  DeleteRegKey /ifempty SHCTX "Software\Classes\.${EXT}"
  ; Explorer remembers the ProgIDs files were opened with from its prompt.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.${EXT}\OpenWithProgids" "Video"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.${EXT}\OpenWithProgids" "Audio"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.${EXT}\OpenWithProgids" "Playlist"
!macroend

; The old ProgIDs, only where they still open ReVLC (the command 0.2.x wrote).
!macro REVLC_DELETE_OLD_PROGID NAME
  ReadRegStr $R0 SHCTX "Software\Classes\${NAME}\shell\open\command" ""
  ${If} $R0 == '${REVLC_EXE} "%1"'
    DeleteRegKey SHCTX "Software\Classes\${NAME}"
  ${EndIf}
!macroend

!macro REVLC_WRITE_PROGID NAME DESCRIPTION
  WriteRegStr SHCTX "Software\Classes\${NAME}" "" "${DESCRIPTION}"
  WriteRegStr SHCTX "Software\Classes\${NAME}" "FriendlyTypeName" "${DESCRIPTION}"
  WriteRegStr SHCTX "Software\Classes\${NAME}\DefaultIcon" "" "${REVLC_EXE},0"
  WriteRegStr SHCTX "Software\Classes\${NAME}\shell" "" "open"
  WriteRegStr SHCTX "Software\Classes\${NAME}\shell\open" "" "Open with ReVLC"
  WriteRegStr SHCTX "Software\Classes\${NAME}\shell\open\command" "" '"${REVLC_EXE}" "%1"'
!macroend

!macro REVLC_REGISTER_TYPE EXT PROGID
  !if "${PROGID}" != ""
    WriteRegStr SHCTX "Software\Classes\.${EXT}\OpenWithProgids" "${PROGID}" ""
    WriteRegStr SHCTX "${REVLC_CAPS}\FileAssociations" ".${EXT}" "${PROGID}"
    WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\SupportedTypes" ".${EXT}" ""
  !endif
!macroend

!macro REVLC_UNREGISTER_TYPE EXT PROGID
  !if "${PROGID}" != ""
    DeleteRegValue SHCTX "Software\Classes\.${EXT}\OpenWithProgids" "${PROGID}"
    DeleteRegKey /ifempty SHCTX "Software\Classes\.${EXT}\OpenWithProgids"
    DeleteRegKey /ifempty SHCTX "Software\Classes\.${EXT}"
  !endif
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro REVLC_FOR_EACH_TYPE REVLC_UNDO_OLD
  !insertmacro REVLC_DELETE_OLD_PROGID "Video"
  !insertmacro REVLC_DELETE_OLD_PROGID "Audio"
  !insertmacro REVLC_DELETE_OLD_PROGID "Playlist"

  !insertmacro REVLC_WRITE_PROGID "ReVLC.Video" "Video"
  !insertmacro REVLC_WRITE_PROGID "ReVLC.Audio" "Audio"
  !insertmacro REVLC_WRITE_PROGID "ReVLC.Playlist" "Playlist"

  ; Open with, and Settings > Default apps.
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe" "FriendlyAppName" "ReVLC"
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\DefaultIcon" "" "${REVLC_EXE},0"
  WriteRegStr SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe\shell\open\command" "" '"${REVLC_EXE}" "%1"'
  WriteRegStr SHCTX "${REVLC_CAPS}" "ApplicationName" "ReVLC"
  WriteRegStr SHCTX "${REVLC_CAPS}" "ApplicationDescription" "A modern media player built on VLC."
  WriteRegStr SHCTX "${REVLC_CAPS}" "ApplicationIcon" "${REVLC_EXE},0"
  WriteRegStr SHCTX "Software\RegisteredApplications" "ReVLC" "${REVLC_CAPS}"
  !insertmacro REVLC_FOR_EACH_TYPE REVLC_REGISTER_TYPE

  !insertmacro UPDATEFILEASSOC
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; An update reinstalls straight away; leave the registration in place.
  ${If} $UpdateMode <> 1
    !insertmacro REVLC_FOR_EACH_TYPE REVLC_UNREGISTER_TYPE
    DeleteRegKey SHCTX "Software\Classes\ReVLC.Video"
    DeleteRegKey SHCTX "Software\Classes\ReVLC.Audio"
    DeleteRegKey SHCTX "Software\Classes\ReVLC.Playlist"
    DeleteRegKey SHCTX "Software\Classes\Applications\${MAINBINARYNAME}.exe"
    DeleteRegValue SHCTX "Software\RegisteredApplications" "ReVLC"
    DeleteRegKey SHCTX "Software\ReVLC"
    !insertmacro UPDATEFILEASSOC
  ${EndIf}
!macroend
