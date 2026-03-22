package com.clay.mudclient;

import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.Switch;
import android.widget.TextView;

import androidx.appcompat.app.AppCompatActivity;

public class SettingsActivity extends AppCompatActivity {
    private static final String PREFS_NAME = "ClayPrefs";
    private static final String KEY_SERVER_HOST = "serverHost";
    private static final String KEY_SERVER_PORT = "serverPort";
    private static final String KEY_USE_SECURE = "useSecure";
    private static final String KEY_SAVED_USERNAME = "savedUsername";
    private static final String KEY_SAVED_PASSWORD = "savedPassword";
    private static final String KEY_ADVANCED_ENABLED = "advancedEnabled";
    private static final String KEY_REMOTE_HOSTNAME = "remoteHostname";
    private static final String KEY_AUTH_KEY = "authKey";

    private EditText serverHostInput;
    private EditText serverPortInput;
    private Switch secureSwitch;
    private CheckBox advancedCheckbox;
    private LinearLayout advancedSection;
    private EditText remoteHostnameInput;
    private EditText serverUsernameInput;
    private EditText serverPasswordInput;
    private TextView connectionStatus;
    private Button saveButton;
    private Button cancelButton;
    private TextView authKeyValue;
    private boolean fromMenu;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_settings);

        serverHostInput = findViewById(R.id.serverHost);
        serverPortInput = findViewById(R.id.serverPort);
        secureSwitch = findViewById(R.id.secureSwitch);
        advancedCheckbox = findViewById(R.id.advancedCheckbox);
        advancedSection = findViewById(R.id.advancedSection);
        remoteHostnameInput = findViewById(R.id.remoteHostname);
        serverUsernameInput = findViewById(R.id.serverUsername);
        serverPasswordInput = findViewById(R.id.serverPassword);
        connectionStatus = findViewById(R.id.connectionStatus);
        saveButton = findViewById(R.id.saveButton);
        cancelButton = findViewById(R.id.cancelButton);
        authKeyValue = findViewById(R.id.authKeyValue);

        fromMenu = getIntent().getBooleanExtra("fromMenu", false);

        // Load saved settings
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String savedHost = prefs.getString(KEY_SERVER_HOST, "192.168.2.6");
        int savedPort = prefs.getInt(KEY_SERVER_PORT, 0);
        boolean savedSecure = prefs.getBoolean(KEY_USE_SECURE, false);
        boolean savedAdvanced = prefs.getBoolean(KEY_ADVANCED_ENABLED, false);
        String savedRemoteHost = prefs.getString(KEY_REMOTE_HOSTNAME, "");
        String savedUsername = prefs.getString(KEY_SAVED_USERNAME, "");
        String savedPassword = prefs.getString(KEY_SAVED_PASSWORD, "");
        String authKey = prefs.getString(KEY_AUTH_KEY, "");

        serverHostInput.setText(savedHost);
        serverPortInput.setText(savedPort > 0 ? String.valueOf(savedPort) : "");
        secureSwitch.setChecked(savedSecure);
        advancedCheckbox.setChecked(savedAdvanced);
        advancedSection.setVisibility(savedAdvanced ? View.VISIBLE : View.GONE);
        remoteHostnameInput.setText(savedRemoteHost);
        serverUsernameInput.setText(savedUsername);
        serverPasswordInput.setText(savedPassword);

        // Display auth key
        if (authKey != null && !authKey.isEmpty()) {
            authKeyValue.setText(authKey);
        } else {
            authKeyValue.setText("none");
            authKeyValue.setTextColor(0xFF484F58);  // dimmer color for "none"
            authKeyValue.setTypeface(null, android.graphics.Typeface.ITALIC);
        }

        // Toggle advanced section visibility
        advancedCheckbox.setOnCheckedChangeListener((buttonView, isChecked) -> {
            advancedSection.setVisibility(isChecked ? View.VISIBLE : View.GONE);
        });

        // Check for error message from MainActivity
        String errorMessage = getIntent().getStringExtra("errorMessage");
        if (errorMessage != null && !errorMessage.isEmpty()) {
            connectionStatus.setText(errorMessage);
            connectionStatus.setVisibility(View.VISIBLE);
        }

        // Update port hint based on secure switch
        updatePortHint();
        secureSwitch.setOnCheckedChangeListener((buttonView, isChecked) -> {
            updatePortHint();
        });

        // Show cancel button only when opened from menu
        if (fromMenu) {
            cancelButton.setVisibility(View.VISIBLE);
            cancelButton.setOnClickListener(v -> finish());
        }

        saveButton.setOnClickListener(v -> saveAndConnect());
    }

    private void updatePortHint() {
        if (secureSwitch.isChecked()) {
            serverPortInput.setHint("9001");
        } else {
            serverPortInput.setHint("9000");
        }
    }

    private void saveAndConnect() {
        String host = serverHostInput.getText().toString().trim();
        String portStr = serverPortInput.getText().toString().trim();
        boolean useSecure = secureSwitch.isChecked();
        boolean advancedEnabled = advancedCheckbox.isChecked();
        String remoteHostname = remoteHostnameInput.getText().toString().trim();
        String username = serverUsernameInput.getText().toString().trim();
        String password = serverPasswordInput.getText().toString();  // Don't trim password

        // Validate inputs
        if (host.isEmpty()) {
            connectionStatus.setText("Please enter a server address");
            connectionStatus.setVisibility(View.VISIBLE);
            return;
        }

        int port;
        if (portStr.isEmpty()) {
            // Use default port based on secure setting
            port = useSecure ? 9001 : 9000;
        } else {
            try {
                port = Integer.parseInt(portStr);
                if (port < 1 || port > 65535) {
                    connectionStatus.setText("Port must be between 1 and 65535");
                    connectionStatus.setVisibility(View.VISIBLE);
                    return;
                }
            } catch (NumberFormatException e) {
                connectionStatus.setText("Invalid port number");
                connectionStatus.setVisibility(View.VISIBLE);
                return;
            }
        }

        // Save settings
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        SharedPreferences.Editor editor = prefs.edit();
        editor.putString(KEY_SERVER_HOST, host);
        editor.putInt(KEY_SERVER_PORT, port);
        editor.putBoolean(KEY_USE_SECURE, useSecure);
        editor.putBoolean(KEY_ADVANCED_ENABLED, advancedEnabled);
        editor.putString(KEY_REMOTE_HOSTNAME, remoteHostname);
        editor.putString(KEY_SAVED_USERNAME, username);
        editor.putString(KEY_SAVED_PASSWORD, password);
        editor.apply();

        // Go back to MainActivity to attempt connection
        Intent intent = new Intent(this, MainActivity.class);
        intent.setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_NEW_TASK);
        startActivity(intent);
        finish();
    }

    @Override
    public void onBackPressed() {
        if (fromMenu) {
            // Opened from menu - back just returns to Clay
            finish();
            return;
        }

        // Initial setup - check if we have valid settings before allowing back
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String host = prefs.getString(KEY_SERVER_HOST, null);
        int port = prefs.getInt(KEY_SERVER_PORT, 0);

        if (host != null && !host.isEmpty() && port > 0) {
            super.onBackPressed();
        } else {
            // No valid settings, can't go back - show message
            connectionStatus.setText("Please enter a server address to continue");
            connectionStatus.setVisibility(View.VISIBLE);
        }
    }
}
