.DEFAULT_GOAL := cli

.PHONY: fake clean cli

ANDROID_DIR := crates/noshtastic-android/android

check:
	cargo check

# NOTE - the `--no-strip` below can be removed to reduce size at the
# cost of non-symbolic backtraces
jni: fake
	cargo ndk \
	  --target arm64-v8a \
	  --no-strip \
	  -o $(ANDROID_DIR)/app/src/main/jniLibs/ \
	  build \
	  --profile dev \
	  --workspace \
	  --exclude noshtastic-cli \
	  --exclude noshtastic-testgw

apk: jni
	cd $(ANDROID_DIR) && ./gradlew build

android: jni
	cd $(ANDROID_DIR) && ./gradlew installDebug
	adb shell am start -n com.bonsai.noshtastic/.MainActivity
	adb logcat -v color -s NOSH

cli:
	cargo build

clean:
	cargo clean
	cd $(ANDROID_DIR) && ./gradlew clean
