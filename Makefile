.DEFAULT_GOAL := check
.PHONY: fake clean

ANDROID_DIR := crates/noshtastic-android/android

check:
	cargo check

tags: fake
	rusty-tags vi

# NOTE - the `--no-strip` below can be removed to reduce size at the
# cost of non-symbolic backtraces
jni: fake
	cargo ndk --target arm64-v8a --no-strip -o $(ANDROID_DIR)/app/src/main/jniLibs/ build --profile dev

apk: jni
	cd $(ANDROID_DIR) && ./gradlew build

android: jni
	cd $(ANDROID_DIR) && ./gradlew installDebug
	adb shell am start -n com.bonsai.noshtastic/.MainActivity
	adb logcat -v color -s NOSH

clean:
	cargo clean
	cd $(ANDROID_DIR) && ./gradlew clean
