# Installer branding

Approved by Jojo: folded-T setup icon, clean Tuck version Setup filename, matching installer branding.

Use existing ICO for NSIS installer and uninstaller. Keep native Windows wizard and installation identity; no custom template or install-path migration. Stage release assets as Tuck_<version>_Setup.exe, retaining signed bytes and matching signature/feed URLs. Accept legacy draft assets during recovery.

Validate release metadata/signature and publishing tests, frontend build, NSIS packaging and embedded installer icon. No release is published by this PR.
