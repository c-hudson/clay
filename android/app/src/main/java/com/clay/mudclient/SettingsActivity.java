package com.clay.mudclient;

import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.Switch;
import android.widget.TextView;

import androidx.appcompat.app.AppCompatActivity;

public class SettingsActivity extends AppCompatActivity {
    private static final String PREFS_NAME = "ClayPrefs";
    private static final String KEY_SERVER_HOST = "serverHost";
    private static final String KEY_SERVER_PORT = "serverPort";
    private static final String KEY_USE_SECURE = "useSecure";

    private EditText serverHostInput;
    private EditText serverPortInput;
    private Switch secureSwitch;
    private TextView connectionStatus;
    private Button saveButton;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_settings);

        serverHostInput = findViewById(R.id.serverHost);
        serverPortInput = findViewById(R.id.serverPort);
        secureSwitch = findViewById(R.id.secureSwitch);
        connectionStatus = findViewById(R.id.connectionStatus);
        saveButton = findViewById(R.id.saveButton);

        // Load saved settings
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String savedHost = prefs.getString(KEY_SERVER_HOST, "");
        int savedPort = prefs.getInt(KEY_SERVER_PORT, 9000);
        boolean savedSecure = prefs.getBoolean(KEY_USE_SECURE, true);

        serverHostInput.setText(savedHost);
        serverPortInput.setText(savedPort > 0 ? String.valueOf(savedPort) : "");
        secureSwitch.setChecked(savedSecure);

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

        saveButton.setOnClickListener(v -> saveAndConnect());
    }

    private void updatePortHint() {
        if (secureSwitch.isChecked()) {
            serverPortInput.setHint("port (default: 9001 for HTTPS)");
        } else {
            serverPortInput.setHint("port (default: 9000 for HTTP)");
        }
    }

    private void saveAndConnect() {
        String host = serverHostInput.getText().toString().trim();
        String portStr = serverPortInput.getText().toString().trim();
        boolean useSecure = secureSwitch.isChecked();

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
        editor.apply();

        // Go back to MainActivity to attempt connection
        Intent intent = new Intent(this, MainActivity.class);
        intent.setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_NEW_TASK);
        startActivity(intent);
        finish();
    }

    @Override
    public void onBackPressed() {
        // Check if we have valid settings before allowing back
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
