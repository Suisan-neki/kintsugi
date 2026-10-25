#!/usr/bin/env bash
# GitHubの一時Ubuntu runner上で、固定したChromiumのOS sandboxを使えるようにする。
set -euo pipefail
sandbox_helper=/opt/google/chrome/chrome-sandbox
if [[ -u "$sandbox_helper" ]] && [[ "$(stat -c %u "$sandbox_helper")" = 0 ]]; then
  echo "CHROME_DEVEL_SANDBOX=$sandbox_helper" >> "$GITHUB_ENV"
  exit 0
fi

# Ubuntuのuser namespace制限をシステム全体では解除しない。
# Playwrightが固定したブラウザの正確なパスだけをAppArmorの例外にする。
# https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md
if [[ -f /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]] &&
   [[ "$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns)" = 1 ]]; then
  browser_path=$(node --input-type=module -e 'import {chromium} from "playwright"; process.stdout.write(chromium.executablePath());')
  [[ -x "$browser_path" && "$browser_path" =~ ^/[a-zA-Z0-9_./-]+$ ]]
  command -v apparmor_parser >/dev/null
  umask 077
  profile_file="$RUNNER_TEMP/mugiurushi-chromium.apparmor"
  printf 'abi <abi/4.0>,\ninclude <tunables/global>\nprofile mugiurushi-chromium "%s" flags=(unconfined) {\n  userns,\n}\n' "$browser_path" > "$profile_file"
  sudo apparmor_parser -r "$profile_file"
fi
