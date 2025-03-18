package com.bonsai.noshtastic;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.graphics.Typeface;
import android.os.Build;
import android.os.Bundle;
import android.text.method.ScrollingMovementMethod;
import android.util.Log;
import android.util.TypedValue;
import android.view.View;
import android.widget.ScrollView;
import android.widget.TextView;

public class MainActivity extends Activity {
    private ScrollView scroll;
	private static TextView logView;

    static {
        // The library name typically replaces dashes with underscores
        System.loadLibrary("noshtastic_android");
    }

    private native void onCreateNative();
    private native void startNoshtastic();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        // Create a ScrollView and TextView dynamically for minimal setup
		scroll = new ScrollView(this);
		logView = new TextView(this);
		logView.setTypeface(Typeface.MONOSPACE);
		logView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 10f);
		logView.setPadding(16, 16, 16, 16);
		logView.setMovementMethod(null);
		scroll.addView(logView);
		setContentView(scroll);

		// this is called on the GUI thread, has good classloader
        onCreateNative();

        // Request BLE permissions on Android S+; otherwise run Rust now.
        requestBluetoothPermissionsIfNeeded();
    }

	private boolean isAtBottom() {
		if (scroll.getChildCount() == 0) return true;
		View child = scroll.getChildAt(0);
		int diff = (child.getBottom() - (scroll.getHeight() + scroll.getScrollY()));
		return (diff <= 100);
	}

    public void appendLog(String line) {
        // Ensure UI update runs on main thread
        logView.post(() -> {
            // 1) Check if we're at (or near) the bottom BEFORE adding text
			boolean wasAtBottom = isAtBottom();

            // 2) Append new line
            logView.append(line + "\n");

            // 3) If we were at the bottom, scroll to show the newly added line
            if (wasAtBottom) {
				scroll.postDelayed(() -> { scroll.smoothScrollTo(0, logView.getBottom()); }, 200);
            }
        });
    }

    private void requestBluetoothPermissionsIfNeeded() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            Log.i("MainActivity", "Requesting BLE permissions");
            requestPermissions(
                new String[] {
                    Manifest.permission.ACCESS_FINE_LOCATION,
                    Manifest.permission.BLUETOOTH_SCAN,
                    Manifest.permission.BLUETOOTH_CONNECT
                },
                1
            );
        } else {
            // On older Android versions, you might need ACCESS_FINE_LOCATION or none at all.
            Log.i("MainActivity", "No run-time BLE permissions needed (SDK < S)");
            startNoshtastic();
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode,
                                           String[] permissions,
                                           int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == 1) {
            boolean allGranted = true;
            for (int result : grantResults) {
                if (result != PackageManager.PERMISSION_GRANTED) {
                    allGranted = false;
                    break;
                }
            }
            if (allGranted) {
                Log.i("MainActivity", "BLE permissions granted");
                startNoshtastic();
            } else {
                Log.e("MainActivity", "BLE permissions denied");
                // Optionally handle or exit gracefully
            }
        }
    }
}
