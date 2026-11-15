#!/usr/bin/env bash
# Proves a signed release APK before it is published: on the emulator it
# installs the published APK, updates it in place with the new one (a phone
# refuses an update signed by another key), starts the app and checks it is
# still running with the new versionName and no crash. Runs inside
# `nix develop .#android-emulator` from the repository root, on a machine with
# /dev/kvm:
#   nix develop .#android-emulator --command android/proof_release.sh OLD.apk NEW.apk
# Paths are relative to the repository root.
set -u
old=$1 new=$2
cd "$(dirname "$0")/.."
export ANDROID_USER_HOME=~/.android ANDROID_AVD_HOME=~/.android/avd
mkdir -p "$ANDROID_AVD_HOME"
export PATH="$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools:$(dirname "$(ls -d "$ANDROID_HOME"/cmdline-tools/*/bin/avdmanager | head -1)"):$PATH"
image=$(ls -d "$ANDROID_HOME"/system-images/*/*/x86_64 | head -1)
package=$(echo "$image" | sed "s#$ANDROID_HOME/##; s#/#;#g")
echo no | avdmanager create avd --force -n release -k "$package" -d pixel_6 >/dev/null 2>&1
emulator -avd release -partition-size 4096 -no-window -no-audio -no-boot-anim -no-snapshot -accel on -gpu swiftshader_indirect >/tmp/emulator.log 2>&1 &
adb wait-for-device
for _ in $(seq 1 120); do [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] && break; sleep 3; done
app=is.libri
fails=0
check() { if eval "$2"; then echo "PASS $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi; }
version() { adb shell dumpsys package $app | grep -m1 versionName | tr -d '\r' | sed 's/.*=//'; }
check "installs $old" 'adb install "$old" 2>&1 | grep -q Success'
echo "installed $(version)"
check "updates in place to $new" 'adb install -r "$new" 2>&1 | tee /dev/stderr | grep -q Success'
echo "now $(version)"
adb logcat -c
adb shell am start -W -n $app/su.libri.MainActivity | grep -E "Status|LaunchState"
sleep 15
adb exec-out screencap -p >android/release-proof/shot.png
check "still running after 15 s" '[ -n "$(adb shell pidof $app | tr -d "\r")" ]'
check "no crash in logcat" '! adb logcat -d -b crash | grep -q "$app"'
adb logcat -d -b crash | tail -20
adb emu kill >/dev/null 2>&1
echo "fails=$fails"
[ "$fails" = 0 ]
