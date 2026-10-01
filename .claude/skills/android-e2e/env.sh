# Source me: `. .claude/skills/android-e2e/env.sh`
# Resolves the Android SDK/NDK roots and puts adb + emulator on PATH without
# assuming where this machine keeps them. Prints what it chose.

_hb_first_dir() { for d in "$@"; do [ -n "$d" ] && [ -d "$d" ] && { printf '%s' "$d"; return 0; }; done; return 1; }

ANDROID_HOME="$(_hb_first_dir "$ANDROID_HOME" "$ANDROID_SDK_ROOT" "$HOME/Library/Android/sdk" "$HOME/Android/Sdk")" \
    || { echo "android-e2e: no Android SDK root found; set ANDROID_HOME" >&2; return 1; }
export ANDROID_HOME ANDROID_SDK_ROOT="$ANDROID_HOME"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"

if [ -z "$ANDROID_NDK_HOME" ] || [ ! -d "$ANDROID_NDK_HOME" ]; then
    ndk_parent="$(_hb_first_dir "$ANDROID_HOME/ndk" /opt/homebrew/share/android-commandlinetools/ndk)"
    [ -n "$ndk_parent" ] && ANDROID_NDK_HOME="$ndk_parent/$(ls -1 "$ndk_parent" | sort -V | tail -1)"
    export ANDROID_NDK_HOME
fi

echo "android-e2e: SDK=$ANDROID_HOME NDK=${ANDROID_NDK_HOME:-<none>}"
echo "android-e2e: AVDs: $(emulator -list-avds 2>/dev/null | tr '\n' ' ')"
echo "android-e2e: rust targets: $(rustup target list --installed 2>/dev/null | grep android | tr '\n' ' ')"
unset -f _hb_first_dir
