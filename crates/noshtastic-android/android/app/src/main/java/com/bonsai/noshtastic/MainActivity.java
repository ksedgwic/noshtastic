package com.bonsai.noshtastic;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.text.method.ScrollingMovementMethod;
import android.util.Log;
import android.util.TypedValue;
import android.view.View;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

public class MainActivity extends Activity {
    private ScrollView scroll;
    private static TextView logView;

    static {
        // The library name typically replaces dashes with underscores
        System.loadLibrary("noshtastic_android");
    }

    private native void onCreateNative();
    private native String[] scanForRadios();
    private native void connectToRadio(String radioName);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

		// This wants to be replaced w/ something more power conservative
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            Intent intent = new Intent(
                Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
				Uri.parse("package:" + getPackageName())
            );
            startActivity(intent);
        }

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
            startBleScan();
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
                startBleScan();
            } else {
                Log.e("MainActivity", "BLE permissions denied");
                Toast.makeText(this, "Bluetooth permissions are required to scan for devices.",
                               Toast.LENGTH_LONG).show();
                finish();
            }
        }
    }

    private void startBleScan() {
        new Thread(() -> {
                String[] localRadioNames = scanForRadios();
                if (localRadioNames == null) {
                    localRadioNames = new String[0];
                }
                if (localRadioNames.length == 1) {
                    // Only one radio: skip dialog and connect directly
                    showLogViewAndConnect(localRadioNames[0]);
                } else {
                    // Multiple or zero radios: show dialog on UI thread
                    final String[] finalRadioNames = localRadioNames;
                    runOnUiThread(() -> showRadioSelectionDialog(finalRadioNames));
                }
        }).start();
    }

    private void showRadioSelectionDialog(String[] radioNames) {
        if (radioNames.length == 0) {
            // No devices found – inform the user and (optionally) retry or exit
            Toast.makeText(this, "No BLE radios found. Please ensure devices are available.",
                           Toast.LENGTH_SHORT).show();
            return;
        }
        // 3. Build a single-choice AlertDialog with the list of radio names
        AlertDialog.Builder builder = new AlertDialog.Builder(this);
        builder.setTitle("Select a Meshtastic Radio");
        builder.setSingleChoiceItems(radioNames, -1, (dialog, which) -> {
            // When a radio name is selected:
            String selectedRadio = radioNames[which];
            dialog.dismiss();  // close the dialog immediately
            // Move to the log view UI and connect to the selected radio
            showLogViewAndConnect(selectedRadio);
        });
        builder.setCancelable(false);  // make dialog modal (require a selection to proceed)
        AlertDialog dialog = builder.create();
        dialog.show();
    }

    private void showLogViewAndConnect(String selectedRadio) {
        new Thread(() -> {
            connectToRadio(selectedRadio);
        }).start();
    }
}
