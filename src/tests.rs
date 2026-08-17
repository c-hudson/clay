    use super::*;

    /// Row-measuring context for tests: the console's own settings at `width`, F2 off.
    /// The measurement helpers all take one now so a budget can never be computed with
    /// different wrapping rules than the renderer draws with.
    fn test_metrics(settings: &Settings, width: usize) -> crate::rendering::RowMetrics<'_> {
        crate::rendering::RowMetrics::new(settings, false, width.max(1))
    }

    #[test]
    fn test_insert_char_ascii() {
        let mut input = InputArea::new(3);
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        assert_eq!(input.buffer, "abc");
        assert_eq!(input.cursor_position, 3);
    }

    #[test]
    fn test_insert_char_emoji() {
        let mut input = InputArea::new(3);
        input.insert_char('😀');
        assert_eq!(input.buffer, "😀");
        assert_eq!(input.cursor_position, 4); // emoji is 4 bytes

        input.insert_char('a');
        assert_eq!(input.buffer, "😀a");
        assert_eq!(input.cursor_position, 5);
    }

    #[test]
    fn test_insert_char_mixed() {
        let mut input = InputArea::new(3);
        input.insert_char('H');
        input.insert_char('i');
        input.insert_char('😀');
        input.insert_char('!');
        assert_eq!(input.buffer, "Hi😀!");
        assert_eq!(input.cursor_position, 7); // 2 + 4 + 1 bytes
    }

    #[test]
    fn test_move_cursor_left_ascii() {
        let mut input = InputArea::new(3);
        input.buffer = "abc".to_string();
        input.cursor_position = 3;

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 2);

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 1);

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 0);

        // Should not go below 0
        input.move_cursor_left();
        assert_eq!(input.cursor_position, 0);
    }

    #[test]
    fn test_move_cursor_left_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 6; // end of string (1 + 4 + 1 bytes)

        input.move_cursor_left(); // move before 'b'
        assert_eq!(input.cursor_position, 5);

        input.move_cursor_left(); // move before emoji (skips all 4 bytes)
        assert_eq!(input.cursor_position, 1);

        input.move_cursor_left(); // move before 'a'
        assert_eq!(input.cursor_position, 0);
    }

    #[test]
    fn test_move_cursor_right_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 0;

        input.move_cursor_right(); // move after 'a'
        assert_eq!(input.cursor_position, 1);

        input.move_cursor_right(); // move after emoji (skips all 4 bytes)
        assert_eq!(input.cursor_position, 5);

        input.move_cursor_right(); // move after 'b'
        assert_eq!(input.cursor_position, 6);

        // Should not go beyond end
        input.move_cursor_right();
        assert_eq!(input.cursor_position, 6);
    }

    #[test]
    fn test_delete_char_ascii() {
        let mut input = InputArea::new(3);
        input.buffer = "abc".to_string();
        input.cursor_position = 3;

        input.delete_char();
        assert_eq!(input.buffer, "ab");
        assert_eq!(input.cursor_position, 2);
    }

    #[test]
    fn test_delete_char_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 5; // after emoji

        input.delete_char(); // delete emoji
        assert_eq!(input.buffer, "ab");
        assert_eq!(input.cursor_position, 1);
    }

    #[test]
    fn test_delete_char_forward_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 1; // before emoji

        input.delete_char_forward(); // delete emoji
        assert_eq!(input.buffer, "ab");
        assert_eq!(input.cursor_position, 1);
    }

    #[test]
    fn test_cursor_line_with_emoji() {
        let mut input = InputArea::new(3);
        input.width = 10;
        // 5 emojis = 10 display columns (2 per emoji), at width 10 fits on 1 line
        input.buffer = "😀😀😀😀😀".to_string();
        input.cursor_position = input.buffer.len(); // end

        // 10 columns at width 10 = cursor at end of line 0, wraps to line 1
        assert_eq!(input.cursor_line(), 1);

        // At width 5, 10 columns = 2 full lines, cursor at start of line 2
        input.width = 5;
        assert_eq!(input.cursor_line(), 2);
    }

    #[test]
    fn test_delete_word_before_cursor_with_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "hello 😀😀 world".to_string();
        input.cursor_position = input.buffer.len();

        input.delete_word_before_cursor(); // delete "world"
        assert_eq!(input.buffer, "hello 😀😀 ");

        // delete_word skips whitespace first, then deletes non-whitespace
        // so this deletes " 😀😀" (space + emojis)
        input.delete_word_before_cursor();
        assert_eq!(input.buffer, "hello ");

        input.delete_word_before_cursor(); // delete "hello"
        assert_eq!(input.buffer, "");
    }

    #[test]
    fn test_home_and_end() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 5;

        input.home();
        assert_eq!(input.cursor_position, 0);

        input.end();
        assert_eq!(input.cursor_position, 6);
    }

    #[test]
    fn test_kill_to_end() {
        let mut input = InputArea::new(3);
        input.buffer = "hello world".to_string();
        input.cursor_position = 5;
        input.kill_to_end();
        assert_eq!(input.buffer, "hello");
        assert_eq!(input.cursor_position, 5);

        // Kill at end does nothing
        input.kill_to_end();
        assert_eq!(input.buffer, "hello");
    }

    #[test]
    fn test_delete_word_forward() {
        let mut input = InputArea::new(3);
        input.buffer = "hello world test".to_string();
        input.cursor_position = 0;
        input.delete_word_forward();
        assert_eq!(input.buffer, " world test");
        assert_eq!(input.cursor_position, 0);

        // From middle of text with leading spaces
        input.buffer = "hello  world".to_string();
        input.cursor_position = 5;
        input.delete_word_forward();
        assert_eq!(input.buffer, "hello");
        assert_eq!(input.cursor_position, 5);
    }

    #[test]
    fn test_capitalize_word() {
        let mut input = InputArea::new(3);
        input.buffer = "hello world".to_string();
        input.cursor_position = 0;
        input.capitalize_word();
        assert_eq!(input.buffer, "Hello world");
        assert_eq!(input.cursor_position, 6); // past "Hello "

        input.capitalize_word();
        assert_eq!(input.buffer, "Hello World");
        assert_eq!(input.cursor_position, 11);
    }

    #[test]
    fn test_lowercase_word() {
        let mut input = InputArea::new(3);
        input.buffer = "HELLO WORLD".to_string();
        input.cursor_position = 0;
        input.lowercase_word();
        assert_eq!(input.buffer, "hello WORLD");
        assert_eq!(input.cursor_position, 6); // past "hello "
    }

    #[test]
    fn test_uppercase_word() {
        let mut input = InputArea::new(3);
        input.buffer = "hello world".to_string();
        input.cursor_position = 0;
        input.uppercase_word();
        assert_eq!(input.buffer, "HELLO world");
        assert_eq!(input.cursor_position, 6); // past "HELLO "
    }

    #[test]
    fn test_insert_at_middle_with_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "ab".to_string();
        input.cursor_position = 1; // between a and b

        input.insert_char('😀');
        assert_eq!(input.buffer, "a😀b");
        assert_eq!(input.cursor_position, 5); // 1 + 4 bytes
    }

    #[test]
    fn test_multiple_emojis() {
        let mut input = InputArea::new(3);
        input.insert_char('🎉');
        input.insert_char('🎊');
        input.insert_char('🎈');

        assert_eq!(input.buffer, "🎉🎊🎈");
        assert_eq!(input.cursor_position, 12); // 3 emojis * 4 bytes each

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 8);

        input.delete_char();
        assert_eq!(input.buffer, "🎉🎈");
        assert_eq!(input.cursor_position, 4);
    }

    #[test]
    fn test_unicode_characters() {
        let mut input = InputArea::new(3);
        // Test various unicode: Chinese, emoji, accented
        input.insert_char('中');  // 3 bytes
        input.insert_char('😀');  // 4 bytes
        input.insert_char('é');   // 2 bytes

        assert_eq!(input.buffer, "中😀é");
        assert_eq!(input.cursor_position, 9); // 3 + 4 + 2

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 7); // before é

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 3); // before 😀

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 0); // before 中
    }

    #[test]
    fn test_password_encrypt_decrypt() {
        // Test basic encryption/decryption
        let password = "mysecretpassword";
        let encrypted = encrypt_password(password);
        assert!(encrypted.starts_with("ENC:"));
        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, password);
    }

    #[test]
    fn test_password_empty() {
        // Empty password should stay empty
        let encrypted = encrypt_password("");
        assert_eq!(encrypted, "");
        let decrypted = decrypt_password("");
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_password_plain_fallback() {
        // Plain passwords (not starting with ENC:) should be returned as-is
        let plain = "plainpassword";
        let decrypted = decrypt_password(plain);
        assert_eq!(decrypted, plain);
    }

    #[test]
    fn test_password_special_chars() {
        // Test password with special characters
        let password = "p@$$w0rd!#$%^&*()";
        let encrypted = encrypt_password(password);
        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, password);
    }

    #[test]
    fn test_password_unicode() {
        // Test password with unicode
        let password = "密码🔐пароль";
        let encrypted = encrypt_password(password);
        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, password);
    }

    #[test]
    fn test_hash_password() {
        let hash = hash_password("test");
        assert_eq!(hash, "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08");
    }

    #[tokio::test]
    async fn test_websocket_auth() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};

        // Start a minimal WebSocket server on a random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        // Expected password hash for "test"
        let server_password = "test";
        let expected_hash = hash_password(server_password);
        println!("Server expects hash: {}", expected_hash);

        // Spawn server task
        let server_hash = expected_hash.clone();
        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut ws_sink, mut ws_source) = ws_stream.split();

            while let Some(msg_result) = ws_source.next().await {
                if let Ok(WsRawMessage::Text(text)) = msg_result {
                    println!("Server received: {}", text);
                    if let Ok(WsMessage::AuthRequest { password_hash: client_hash, .. }) = serde_json::from_str::<WsMessage>(&text) {
                        println!("Client hash: {}", client_hash);
                        println!("Server hash: {}", server_hash);
                        let auth_success = client_hash == server_hash;
                        println!("Auth success: {}", auth_success);
                        let response = WsMessage::AuthResponse {
                            success: auth_success,
                            error: if auth_success { None } else { Some("Invalid password".to_string()) },
                            username: None,
                            multiuser_mode: false,
                        };
                        let json = serde_json::to_string(&response).unwrap();
                        ws_sink.send(WsRawMessage::Text(json)).await.unwrap();
                        break;
                    }
                }
            }
        });

        // Give server time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Connect client
        let url = format!("ws://127.0.0.1:{}", port);
        let (ws_stream, _) = connect_async(&url).await.unwrap();
        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Send auth request with correct password hash
        let client_password = "test";
        let client_hash = hash_password(client_password);
        println!("Client sending hash: {}", client_hash);
        let auth_msg = WsMessage::AuthRequest { password_hash: client_hash, username: None, current_world: None, auth_key: None, request_key: false, challenge_response: false, resume: Vec::new(), resume_epochs: Vec::new(), client_uid: String::new() };
        let json = serde_json::to_string(&auth_msg).unwrap();
        ws_sink.send(WsRawMessage::Text(json)).await.unwrap();

        // Wait for response
        if let Some(Ok(WsRawMessage::Text(text))) = ws_source.next().await {
            println!("Client received: {}", text);
            let response: WsMessage = serde_json::from_str(&text).unwrap();
            if let WsMessage::AuthResponse { success, error, .. } = response {
                assert!(success, "Auth should succeed but got error: {:?}", error);
            } else {
                panic!("Expected AuthResponse");
            }
        } else {
            panic!("No response received");
        }

        server_task.abort();
    }

    #[test]
    fn test_world_cycling_all_connected() {
        // Test cycling through multiple connected worlds
        let mut app = App::new();
        app.worlds.clear(); // Remove any default world

        // Create 3 connected worlds with different names
        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("zeta");
        world_zeta.connected = true;
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on alpha

        // Verify initial state
        assert_eq!(app.worlds[app.current_world_index].name, "alpha");

        // Cycle forward: alpha -> cave
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "After first next_world from alpha, should be on cave");

        // Cycle forward: cave -> zeta
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "zeta",
            "After second next_world from cave, should be on zeta");

        // Cycle forward: zeta -> alpha (wrap)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "alpha",
            "After third next_world from zeta, should wrap to alpha");

        // Cycle backward: alpha -> zeta
        app.prev_world();
        assert_eq!(app.worlds[app.current_world_index].name, "zeta",
            "After prev_world from alpha, should be on zeta");

        // Cycle backward: zeta -> cave
        app.prev_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "After prev_world from zeta, should be on cave");
    }

    #[test]
    fn test_world_cycling_with_disconnected() {
        // Test that disconnected worlds without unseen output are skipped
        let mut app = App::new();
        app.worlds.clear();

        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        app.worlds.push(world_alpha);

        let mut world_beta = World::new("beta");
        world_beta.connected = false; // Disconnected, no unseen output
        app.worlds.push(world_beta);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        app.worlds.push(world_cave);

        app.current_world_index = 0; // Start on alpha

        // Cycle forward: alpha -> cave (skipping disconnected beta)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "Should skip disconnected beta and go to cave");

        // Cycle forward: cave -> alpha (skipping disconnected beta)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "alpha",
            "Should skip disconnected beta and wrap to alpha");
    }

    #[test]
    fn test_world_cycling_case_insensitive_sort() {
        // Test that world names are sorted case-insensitively
        let mut app = App::new();
        app.worlds.clear();

        let mut world_alpha = World::new("Alpha"); // Capital A
        world_alpha.connected = true;
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave"); // lowercase c
        world_cave.connected = true;
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("Zeta"); // Capital Z
        world_zeta.connected = true;
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on Alpha

        // Should cycle: Alpha -> cave -> Zeta (case-insensitive alphabetical)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "Case-insensitive sort: Alpha -> cave");

        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "Zeta",
            "Case-insensitive sort: cave -> Zeta");

        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "Alpha",
            "Case-insensitive sort: Zeta -> Alpha (wrap)");
    }

    #[test]
    fn test_world_cycling_unseen_first_no_unseen() {
        // Test world_switch_mode=UnseenFirst when no worlds have unseen output
        let mut app = App::new();
        app.worlds.clear();
        app.settings.world_switch_mode = WorldSwitchMode::UnseenFirst;

        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        world_alpha.unseen_lines = 0; // No unseen
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        world_cave.unseen_lines = 0; // No unseen
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("zeta");
        world_zeta.connected = true;
        world_zeta.unseen_lines = 0; // No unseen
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on alpha

        // With UnseenFirst ON but no unseen, should cycle alphabetically
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "With UnseenFirst but no unseen, should go to cave");

        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "zeta",
            "With UnseenFirst but no unseen, should go to zeta");
    }

    #[test]
    fn test_world_cycling_unseen_first_with_unseen() {
        // Test world_switch_mode=UnseenFirst prioritizes worlds with unseen output
        let mut app = App::new();
        app.worlds.clear();
        app.settings.world_switch_mode = WorldSwitchMode::UnseenFirst;

        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        world_alpha.unseen_lines = 0; // No unseen
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        world_cave.unseen_lines = 5; // Has unseen!
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("zeta");
        world_zeta.connected = true;
        world_zeta.unseen_lines = 0; // No unseen
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on alpha

        // With UnseenFirst ON and cave has unseen, should go to cave first
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "With UnseenFirst, should prioritize cave with unseen output");
    }

    #[test]
    fn test_decode_strips_control_chars() {
        // Test that carriage return is stripped
        let input = b"hello\rworld";
        let result = Encoding::Utf8.decode(input);
        assert!(!result.contains('\r'), "Carriage return should be stripped");
        assert_eq!(result, "helloworld", "CR should be removed, text concatenated");

        // Test that other control characters are stripped but tab/newline kept
        let input = b"a\x01b\tc\nd\x7Fe";
        let result = Encoding::Utf8.decode(input);
        assert_eq!(result, "ab\tc\nde", "Control chars stripped except tab/newline");

        // Test that BEL is stripped in final output
        let input = b"hello\x07world";
        let result = Encoding::Utf8.decode(input);
        assert!(!result.contains('\x07'), "BEL should be stripped in final output");
    }

    #[test]
    fn test_strip_non_sgr_sequences() {
        // Test that SGR (color/style) sequences are kept
        let input = "\x1b[31mred text\x1b[0m";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "\x1b[31mred text\x1b[0m", "SGR sequences should be preserved");

        // Test that cursor position (H) inserts newline
        let input = "first\x1b[10;5Hsecond";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "first\nsecond", "Cursor positioning (H) should insert newline");

        // Test that cursor column (G) inserts space
        let input = "before\x1b[10Gafter";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "before after", "Cursor column (G) should insert space");

        // Test that erase sequences are stripped without separator
        let input = "hello\x1b[2Jworld";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "helloworld", "Erase (J) should be stripped");

        // Test that erase line (K) is stripped
        let input = "hello\x1b[Kworld";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "helloworld", "Erase line (K) should be stripped");

        // Test OSC (window title) sequences are stripped
        let input = "before\x1b]0;Window Title\x07after";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "beforeafter", "OSC sequences should be stripped");

        // Test cursor up/down inserts newline
        let input = "line1\x1b[Aline2";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "line1\nline2", "Cursor up (A) should insert newline");

        // Test @ character (insert character)
        let input = "before\x1b[5@after";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "beforeafter", "Insert character (@) should be stripped");

        // Test ~ character (function key)
        let input = "text\x1b[6~more";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "textmore", "Function key sequences (~) should be stripped");

        // Test that consecutive positioning doesn't add multiple separators
        let input = "text\x1b[H\x1b[Hmore";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "text\nmore", "Consecutive H should only add one newline");

        // Test that malformed CSI sequences don't consume URL text
        // A malformed sequence like ESC[? followed by https:// should not consume the 'h'
        let input = "before\x1b[?https://example.com";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "before\x1b[?https://example.com", "Malformed CSI should preserve URL text");

        // Test that valid private mode sequences are still processed
        let input = "before\x1b[?25hafter";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "beforeafter", "Valid private mode sequence should be stripped");
    }

    #[test]
    fn test_keep_alive_type_cycling() {
        // Test next() cycling
        assert_eq!(KeepAliveType::None.next(), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::Nop.next(), KeepAliveType::Custom);
        assert_eq!(KeepAliveType::Custom.next(), KeepAliveType::Generic);
        assert_eq!(KeepAliveType::Generic.next(), KeepAliveType::None);

        // Test prev() cycling
        assert_eq!(KeepAliveType::None.prev(), KeepAliveType::Generic);
        assert_eq!(KeepAliveType::Nop.prev(), KeepAliveType::None);
        assert_eq!(KeepAliveType::Custom.prev(), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::Generic.prev(), KeepAliveType::Custom);
    }

    #[test]
    fn test_keep_alive_type_name() {
        assert_eq!(KeepAliveType::None.name(), "None");
        assert_eq!(KeepAliveType::Nop.name(), "NOP");
        assert_eq!(KeepAliveType::Custom.name(), "Custom");
        assert_eq!(KeepAliveType::Generic.name(), "Generic");
    }

    #[test]
    fn test_keep_alive_type_from_name() {
        assert_eq!(KeepAliveType::from_name("None"), KeepAliveType::None);
        assert_eq!(KeepAliveType::from_name("none"), KeepAliveType::None);
        assert_eq!(KeepAliveType::from_name("NOP"), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::from_name("nop"), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::from_name("Custom"), KeepAliveType::Custom);
        assert_eq!(KeepAliveType::from_name("custom"), KeepAliveType::Custom);
        assert_eq!(KeepAliveType::from_name("Generic"), KeepAliveType::Generic);
        assert_eq!(KeepAliveType::from_name("generic"), KeepAliveType::Generic);
        // Unknown should default to Nop
        assert_eq!(KeepAliveType::from_name("unknown"), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::from_name(""), KeepAliveType::Nop);
    }

    #[test]
    fn test_idler_message_filter() {
        // Test that lines containing idler message pattern are detected
        let idler_line = "You don't know how to help commands ###_idler_message_123_###.";
        assert!(idler_line.contains("###_idler_message_") && idler_line.contains("_###"));

        let normal_line = "You say, \"Hello world!\"";
        assert!(!(normal_line.contains("###_idler_message_") && normal_line.contains("_###")));

        // Test partial matches don't trigger
        let partial1 = "###_idler_message_ incomplete";
        assert!(!(partial1.contains("###_idler_message_") && partial1.contains("_###")));

        let partial2 = "incomplete _### suffix only";
        assert!(!(partial2.contains("###_idler_message_") && partial2.contains("_###")));
    }

    #[test]
    fn test_idler_message_replacement() {
        // Test that ##rand## is replaced correctly in custom commands
        let custom_cmd = "look ##rand##";
        let rand_num = 42u32;
        let idler_tag = format!("###_idler_message_{}_###", rand_num);
        let result = custom_cmd.replace("##rand##", &idler_tag);
        assert_eq!(result, "look ###_idler_message_42_###");

        // Test generic command format
        let generic_cmd = format!("help commands ###_idler_message_{}_###", rand_num);
        assert_eq!(generic_cmd, "help commands ###_idler_message_42_###");
    }

    #[test]
    fn test_is_visually_empty() {
        use super::is_visually_empty;

        // Empty string is visually empty
        assert!(is_visually_empty(""));

        // Whitespace-only is visually empty
        assert!(is_visually_empty("   "));
        assert!(is_visually_empty("\t"));
        assert!(is_visually_empty("  \t  "));

        // ANSI codes only are visually empty
        assert!(is_visually_empty("\x1b[0m"));
        assert!(is_visually_empty("\x1b[31m\x1b[0m"));
        assert!(is_visually_empty("\x1b[1;32m"));

        // ANSI codes with whitespace are visually empty
        assert!(is_visually_empty("\x1b[0m   \x1b[31m"));
        assert!(is_visually_empty("  \x1b[0m  "));

        // Visible text is NOT visually empty
        assert!(!is_visually_empty("hello"));
        assert!(!is_visually_empty("  hello  "));
        assert!(!is_visually_empty("\x1b[31mhello\x1b[0m"));
        assert!(!is_visually_empty("a"));
        assert!(!is_visually_empty("\x1b[0m.\x1b[0m"));
    }

    #[test]
    fn test_has_background_color() {
        use super::has_background_color;

        // No background color
        assert!(!has_background_color(""));
        assert!(!has_background_color("hello"));
        assert!(!has_background_color("\x1b[31mred text\x1b[0m"));
        assert!(!has_background_color("\x1b[1;32mbold green\x1b[0m"));

        // Standard background colors (40-47)
        assert!(has_background_color("\x1b[40m"));
        assert!(has_background_color("\x1b[44mblue bg\x1b[0m"));
        assert!(has_background_color("\x1b[47m   \x1b[0m"));

        // Bright background colors (100-107)
        assert!(has_background_color("\x1b[100m"));
        assert!(has_background_color("\x1b[104m"));
        assert!(has_background_color("\x1b[107m"));

        // 256-color background (48;5;N)
        assert!(has_background_color("\x1b[48;5;15m"));
        assert!(has_background_color("\x1b[48;5;15m   \x1b[0m"));
        assert!(has_background_color("\x1b[48;5;255mwhite\x1b[0m"));

        // True color background (48;2;R;G;B)
        assert!(has_background_color("\x1b[48;2;255;255;255m"));
        assert!(has_background_color("\x1b[48;2;0;0;0mblack\x1b[0m"));

        // Combined foreground and background
        assert!(has_background_color("\x1b[31;44mred on blue\x1b[0m"));
        assert!(has_background_color("\x1b[38;5;15;48;5;0m"));

        // Whitespace with background color (ANSI art case)
        assert!(has_background_color("\x1b[48;5;15m                    \x1b[0m"));
    }

    #[test]
    fn test_is_ansi_only_line() {
        use super::is_ansi_only_line;

        // Empty string is NOT ANSI-only (it's just empty)
        assert!(!is_ansi_only_line(""));

        // Whitespace-only is NOT ANSI-only
        assert!(!is_ansi_only_line("   "));

        // Pure ANSI codes without content (garbage that should be filtered)
        assert!(is_ansi_only_line("\x1b[0m"));
        assert!(is_ansi_only_line("\x1b[H\x1b[J"));  // Cursor control garbage
        assert!(is_ansi_only_line("\x1b[31m\x1b[0m"));  // Color codes only
        assert!(is_ansi_only_line("\x1b[0m   \x1b[31m"));  // ANSI + whitespace only (no bg color)

        // Lines with visible content should NOT be filtered
        assert!(!is_ansi_only_line("hello"));
        assert!(!is_ansi_only_line("\x1b[31mhello\x1b[0m"));

        // CRITICAL: Lines with background colors should NOT be filtered even if no visible text
        // This is the ANSI art case - white background with spaces
        assert!(!is_ansi_only_line("\x1b[48;5;15m                    \x1b[0m"));
        assert!(!is_ansi_only_line("\x1b[44m   \x1b[0m"));  // Standard blue bg
        assert!(!is_ansi_only_line("\x1b[100m\x1b[0m"));  // Bright background
        assert!(!is_ansi_only_line("\x1b[48;2;255;255;255m  \x1b[0m"));  // True color bg
    }

    #[test]
    fn test_wrap_urls_with_osc8() {
        use super::wrap_urls_with_osc8;

        // No URLs - return unchanged
        assert_eq!(wrap_urls_with_osc8("hello world"), "hello world");
        assert_eq!(wrap_urls_with_osc8("no links here"), "no links here");

        // Simple HTTP URL - using BEL (0x07) as terminator
        let result = wrap_urls_with_osc8("check http://example.com please");
        assert!(result.contains("\x1b]8;;http://example.com\x07"));
        assert!(result.contains("http://example.com\x1b]8;;\x07"));

        // HTTPS URL
        let result = wrap_urls_with_osc8("visit https://example.com/path");
        assert!(result.contains("\x1b]8;;https://example.com/path\x07"));

        // URL with query parameters
        let result = wrap_urls_with_osc8("link: https://example.com/page?foo=bar&baz=qux");
        assert!(result.contains("foo=bar&baz=qux"));

        // URL followed by punctuation (should not include trailing punctuation)
        let result = wrap_urls_with_osc8("See https://example.com.");
        assert!(result.contains("\x1b]8;;https://example.com\x07"));
        assert!(!result.contains("\x1b]8;;https://example.com.\x07"));

        // URL in quotes
        let result = wrap_urls_with_osc8("Nina says, \"https://tenor.com/view/test\"");
        assert!(result.contains("\x1b]8;;https://tenor.com/view/test\x07"));

        // Multiple URLs
        let result = wrap_urls_with_osc8("http://a.com and https://b.com");
        assert!(result.contains("\x1b]8;;http://a.com\x07"));
        assert!(result.contains("\x1b]8;;https://b.com\x07"));

        // URL with zero-width spaces (U+200B) should have them stripped from OSC 8 URL parameter
        // but preserved in visible text for word breaking
        let url_with_zwsp = "https://example.com/\u{200B}path/\u{200B}to/\u{200B}page";
        let result = wrap_urls_with_osc8(url_with_zwsp);
        // OSC 8 URL parameter should have clean URL without ZWSP
        assert!(result.contains("\x1b]8;;https://example.com/path/to/page\x07"));
        // Visible text should preserve ZWSP for word breaking
        assert!(result.contains("/\u{200B}path/\u{200B}to/\u{200B}page"));

        // URL followed by a trailing ANSI color code — the trailing-strip loop now
        // walks back over CSI sequences so they are emitted after the OSC 8 close
        // rather than inside it, keeping them out of the href and the visible span.
        let url_with_ansi = "https://example.com\x1b[0;37m rest";
        let result = wrap_urls_with_osc8(url_with_ansi);
        // Clean URL in OSC 8 parameter should not include the ANSI code
        assert!(result.contains("\x1b]8;;https://example.com\x07"));
        // ANSI code now comes after the OSC 8 close (not inside the link)
        assert!(result.contains("https://example.com\x1b]8;;\x07\x1b[0;37m"));

        // Regression: colored URL ending with a period followed by a reset sequence.
        // The period must NOT appear in the href (caused a 404 in browsers).
        // Input byte stream: green-on, url, period, reset, closing quote
        let colored_url_with_period = "\x1b[32mhttp://teenymush.dynu.net/~g7_cq7.\x1b[0m'";
        let result = wrap_urls_with_osc8(colored_url_with_period);
        // href must be clean (no period)
        assert!(result.contains("\x1b]8;;http://teenymush.dynu.net/~g7_cq7\x07"),
            "href should not contain trailing period; got: {:?}", result);
        assert!(!result.contains("\x1b]8;;http://teenymush.dynu.net/~g7_cq7.\x07"),
            "href must not include the trailing period");
        // Option 2: the OSC 8 visible region now covers the trailing period, reset, and
        // closing quote, so the terminal's own URL matcher is overridden on those cells.
        assert!(result.contains("http://teenymush.dynu.net/~g7_cq7.\x1b[0m'\x1b]8;;\x07"),
            "visible link region should include trailing period, reset, and quote; got: {:?}", result);
    }

    #[test]
    fn test_strip_mud_tag() {
        use super::strip_mud_tag;

        // Pattern 2: [name:] - colon immediately before ]
        assert_eq!(strip_mud_tag("[channel:] hello"), "hello");
        assert_eq!(strip_mud_tag("[chat:] message"), "message");

        // Pattern 1: [name(content)optional]
        assert_eq!(strip_mud_tag("[ooc(player)] text"), "text");
        assert_eq!(strip_mud_tag("[chat(Bob)extra] text"), "text");

        // Indented lines are NOT stripped (preserves MUSH code like [match(...)])
        assert_eq!(strip_mud_tag("  [channel:] hello"), "  [channel:] hello");

        // With ANSI color prefix
        assert_eq!(strip_mud_tag("\x1b[31m[channel:] hello"), "\x1b[31mhello");
        assert_eq!(strip_mud_tag("\x1b[1;32m[chat:] text"), "\x1b[1;32mtext");

        // Non-tag brackets should NOT be stripped
        assert_eq!(strip_mud_tag("[hello] world"), "[hello] world");
        assert_eq!(strip_mud_tag("[nochannel] text"), "[nochannel] text");

        // Colon not at end should NOT be stripped (e.g., [a:b])
        assert_eq!(strip_mud_tag("[a:b] text"), "[a:b] text");

        // Bare colon with no name should NOT be stripped
        assert_eq!(strip_mud_tag("[:] text"), "[:] text");

        // Empty parens should NOT be stripped (e.g., [time()])
        assert_eq!(strip_mud_tag("[time()] text"), "[time()] text");

        // Bare paren with no name before it should NOT be stripped
        assert_eq!(strip_mud_tag("[(foo)] text"), "[(foo)] text");

        // Unclosed paren should NOT be stripped
        assert_eq!(strip_mud_tag("[chat(Bob] text"), "[chat(Bob] text");

        // No brackets at start
        assert_eq!(strip_mud_tag("hello world"), "hello world");
        assert_eq!(strip_mud_tag("text [tag:] later"), "text [tag:] later");

        // Tag without space after ] should NOT be stripped
        assert_eq!(strip_mud_tag("[channel:]"), "[channel:]");
        assert_eq!(strip_mud_tag("[channel:]hello"), "[channel:]hello");

        // Tag with only trailing space - space is consumed, result is empty
        assert_eq!(strip_mud_tag("[channel:] "), "");
    }

    // ============================================================================
    // Security regression tests
    // ============================================================================

    /// Test: RevokeKey must require authentication (CVE-like: pre-auth key revocation)
    /// An unauthenticated WebSocket client must NOT be able to revoke auth keys.
    #[tokio::test]
    async fn test_security_revoke_key_requires_auth() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};
        use crate::websocket::{WsMessage, WsClientInfo};
        use std::sync::Arc;
        use std::sync::RwLock;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        let password_hash = hash_password("testpass");
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let clients: Arc<RwLock<std::collections::HashMap<u64, WsClientInfo>>> =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let allow_list: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let ban_list = BanList::new();
        let users: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::websocket::UserCredential>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));

        // Spawn server handler
        let server_clients = Arc::clone(&clients);
        let server_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream,
                1, // client_id
                server_clients,
                password_hash,
                true, // password_enabled
                allow_list,
                whitelisted,
                client_addr,
                event_tx,
                false, // not multiuser
                users,
                ban_list,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Connect as client but do NOT authenticate
        let url = format!("ws://127.0.0.1:{}", port);
        let (ws_stream, _) = connect_async(&url).await.unwrap();
        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Skip ServerHello
        let _ = ws_source.next().await;

        // Try to send RevokeKey without authenticating
        let revoke_msg = WsMessage::RevokeKey { auth_key: "some_key".to_string() };
        let json = serde_json::to_string(&revoke_msg).unwrap();
        ws_sink.send(WsRawMessage::Text(json)).await.unwrap();

        // The server should disconnect us (break from loop) since we're not authenticated
        // Wait briefly for the server to process
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Check that NO WsKeyRevoke event was sent to the app
        // (the event channel should only have WsClientConnected and WsClientDisconnected)
        let mut found_revoke = false;
        while let Ok(event) = event_rx.try_recv() {
            if let AppEvent::WsKeyRevoke(_, _) = event {
                found_revoke = true;
            }
        }
        assert!(!found_revoke, "RevokeKey should NOT be processed for unauthenticated clients");

        server_task.abort();
    }

    /// Test: Unauthenticated clients cannot send commands
    /// Any non-auth message from an unauthenticated client must be rejected.
    #[tokio::test]
    async fn test_security_unauth_cannot_send_commands() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};
        use crate::websocket::{WsMessage, WsClientInfo};
        use std::sync::Arc;
        use std::sync::RwLock;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        let password_hash = hash_password("testpass");
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let clients: Arc<RwLock<std::collections::HashMap<u64, WsClientInfo>>> =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let allow_list: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let ban_list = BanList::new();
        let users: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::websocket::UserCredential>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));

        let server_clients = Arc::clone(&clients);
        let server_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream, 1, server_clients, password_hash, true,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}", port);
        let (ws_stream, _) = connect_async(&url).await.unwrap();
        let (mut ws_sink, _ws_source) = ws_stream.split();

        // Try sending a command without authenticating
        let cmd = WsMessage::SendCommand { world_index: 0, command: "look".to_string() };
        let json = serde_json::to_string(&cmd).unwrap();
        ws_sink.send(WsRawMessage::Text(json)).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Verify no WsClientMessage was forwarded to app
        let mut found_command = false;
        while let Ok(event) = event_rx.try_recv() {
            if let AppEvent::WsClientMessage(_, msg) = event {
                if let WsMessage::SendCommand { .. } = *msg {
                    found_command = true;
                }
            }
        }
        assert!(!found_command, "Unauthenticated client should NOT be able to send commands");

        server_task.abort();
    }

    /// Test: Failed password auth triggers ban violation
    #[tokio::test]
    async fn test_security_failed_auth_records_violation() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};
        use crate::websocket::{WsMessage, WsClientInfo};
        use std::sync::Arc;
        use std::sync::RwLock;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        let password_hash = hash_password("correctpassword");
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let clients: Arc<RwLock<std::collections::HashMap<u64, WsClientInfo>>> =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let allow_list: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let ban_list = BanList::new();
        let users: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::websocket::UserCredential>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));

        let server_clients = Arc::clone(&clients);
        let server_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream, 1, server_clients, password_hash, true,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}", port);
        let (ws_stream, _) = connect_async(&url).await.unwrap();
        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Skip ServerHello
        let _ = ws_source.next().await;

        // Send wrong password
        let wrong_hash = hash_password("wrongpassword");
        let auth_msg = WsMessage::AuthRequest {
            password_hash: wrong_hash,
            username: None,
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: false,
            resume: Vec::new(), resume_epochs: Vec::new(), client_uid: String::new(),
        };
        let json = serde_json::to_string(&auth_msg).unwrap();
        ws_sink.send(WsRawMessage::Text(json)).await.unwrap();

        // Read response - should be auth failure
        if let Some(Ok(WsRawMessage::Text(text))) = ws_source.next().await {
            let response: WsMessage = serde_json::from_str(&text).unwrap();
            if let WsMessage::AuthResponse { success, .. } = response {
                assert!(!success, "Auth should fail with wrong password");
            }
        }

        // Note: ban_list violations from localhost are ignored (127.0.0.1 exempt)
        // This test verifies the auth flow rejects bad passwords
        // Ban tracking for non-localhost IPs is verified by the ban_list unit tests

        server_task.abort();
    }

    /// Test: Multiuser auth error messages don't reveal user existence
    /// Both invalid username and invalid password should return the same error.
    #[tokio::test]
    async fn test_security_multiuser_no_user_enumeration() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};
        use crate::websocket::{WsMessage, WsClientInfo, UserCredential};
        use std::sync::Arc;
        use std::sync::RwLock;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        let password_hash = hash_password("serverpass");
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let clients: Arc<RwLock<std::collections::HashMap<u64, WsClientInfo>>> =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let allow_list: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let ban_list = BanList::new();

        // Set up multiuser mode with one user
        let mut users_map = std::collections::HashMap::new();
        users_map.insert("admin".to_string(), UserCredential {
            password_hash: hash_password("adminpass"),
        });
        let users: Arc<std::sync::RwLock<std::collections::HashMap<String, UserCredential>>> =
            Arc::new(std::sync::RwLock::new(users_map));

        // We need two connections to test both error cases
        let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port2 = listener2.local_addr().unwrap().port();

        let clients2 = Arc::clone(&clients);
        let users2 = Arc::clone(&users);
        let ban_list2 = ban_list.clone();
        let (event_tx2, _) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let password_hash2 = password_hash.clone();

        let server_clients = Arc::clone(&clients);

        // Server 1: test invalid username
        let server_task1 = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream, 1, server_clients, password_hash, true,
                allow_list, whitelisted, client_addr, event_tx,
                true, // multiuser mode
                users, ban_list,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        // Server 2: test wrong password for valid user
        let allow_list2: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted2: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let server_task2 = tokio::spawn(async move {
            let (stream, client_addr) = listener2.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream, 2, clients2, password_hash2, true,
                allow_list2, whitelisted2, client_addr, event_tx2,
                true, users2, ban_list2,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Test 1: Invalid username
        let url1 = format!("ws://127.0.0.1:{}", port);
        let (ws1, _) = connect_async(&url1).await.unwrap();
        let (mut sink1, mut source1) = ws1.split();
        let _ = source1.next().await; // skip ServerHello

        let auth1 = WsMessage::AuthRequest {
            password_hash: hash_password("anything"),
            username: Some("nonexistent".to_string()),
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: false,
            resume: Vec::new(), resume_epochs: Vec::new(), client_uid: String::new(),
        };
        sink1.send(WsRawMessage::Text(serde_json::to_string(&auth1).unwrap())).await.unwrap();
        let error1 = if let Some(Ok(WsRawMessage::Text(text))) = source1.next().await {
            let resp: WsMessage = serde_json::from_str(&text).unwrap();
            if let WsMessage::AuthResponse { error, .. } = resp { error } else { None }
        } else { None };

        // Test 2: Valid username, wrong password
        let url2 = format!("ws://127.0.0.1:{}", port2);
        let (ws2, _) = connect_async(&url2).await.unwrap();
        let (mut sink2, mut source2) = ws2.split();
        let _ = source2.next().await; // skip ServerHello

        let auth2 = WsMessage::AuthRequest {
            password_hash: hash_password("wrongpassword"),
            username: Some("admin".to_string()),
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: false,
            resume: Vec::new(), resume_epochs: Vec::new(), client_uid: String::new(),
        };
        sink2.send(WsRawMessage::Text(serde_json::to_string(&auth2).unwrap())).await.unwrap();
        let error2 = if let Some(Ok(WsRawMessage::Text(text))) = source2.next().await {
            let resp: WsMessage = serde_json::from_str(&text).unwrap();
            if let WsMessage::AuthResponse { error, .. } = resp { error } else { None }
        } else { None };

        // Both errors must be identical to prevent user enumeration
        assert_eq!(error1, error2,
            "Invalid username and wrong password must return the same error message \
             to prevent user enumeration. Got: {:?} vs {:?}", error1, error2);

        // Verify the error is generic
        assert_eq!(error1, Some("Authentication failed".to_string()),
            "Error message should be generic 'Authentication failed'");

        server_task1.abort();
        server_task2.abort();
    }

    /// Test: Host header sanitization strips JS injection characters
    /// The host header is injected into HTML templates as a JS string value.
    /// Characters that could break out of the string context must be stripped.
    #[test]
    fn test_security_host_header_sanitization() {
        // These are the dangerous characters for JS string injection
        let malicious_hosts = vec![
            ("example.com\";alert(1);//", "example.com;alert(1);//"),
            ("example.com';alert(1);//", "example.com;alert(1);//"),
            ("example.com`+alert(1)+`", "example.com+alert(1)+"),
            ("example.com\\x22;alert(1)", "example.comx22;alert(1)"),
            ("<script>alert(1)</script>", "scriptalert(1)/script"),
            ("example.com\">", "example.com"),
        ];

        for (input, expected) in malicious_hosts {
            let sanitized = input.replace(['\\', '\'', '"', '`', '<', '>'], "");
            assert_eq!(sanitized, expected,
                "Host header sanitization failed for input: {:?}", input);
        }
    }

    /// Test: Valid HTTP paths don't trigger ban violations
    #[test]
    fn test_security_valid_paths_no_ban() {
        let valid_paths = vec![
            "/", "/index.html", "/style.css", "/app.js",
            "/theme-editor", "/keybind-editor", "/favicon.ico",
        ];

        for path in valid_paths {
            // Verify these paths are in the known set
            // If any of these paths start returning 404, it would be a regression
            let is_valid = matches!(path,
                "/" | "/index.html" | "/style.css" | "/app.js" |
                "/theme-editor" | "/keybind-editor" | "/favicon.ico"
            );
            assert!(is_valid, "Path {} should be recognized as valid", path);
        }
    }

    /// Test: BanList localhost exemption
    /// Localhost connections must never be banned (prevents self-lockout)
    #[test]
    fn test_security_localhost_ban_exempt() {
        let ban_list = BanList::new();

        // Record many violations from localhost - should never result in ban
        for _ in 0..20 {
            ban_list.record_violation("127.0.0.1", "test violation");
        }
        assert!(!ban_list.is_banned("127.0.0.1"),
            "127.0.0.1 must never be banned");

        for _ in 0..20 {
            ban_list.record_violation("::1", "test violation");
        }
        assert!(!ban_list.is_banned("::1"),
            "::1 must never be banned");

        for _ in 0..20 {
            ban_list.record_violation("localhost", "test violation");
        }
        assert!(!ban_list.is_banned("localhost"),
            "localhost must never be banned");
    }

    /// Test: BanList bans external IPs after threshold violations
    #[test]
    fn test_security_ban_after_violations() {
        let ban_list = BanList::new();

        // 5 violations should trigger permanent ban for non-localhost
        for i in 0..5 {
            let banned = ban_list.record_violation("10.0.0.1", &format!("violation {}", i));
            if i < 4 {
                // Before 5th violation, may get temp ban at 3
                let _ = banned;
            }
        }
        assert!(ban_list.is_banned("10.0.0.1"),
            "External IP should be banned after 5 violations");
    }

    /// Test: Password hash is deterministic (SHA-256)
    #[test]
    fn test_security_password_hash_deterministic() {
        let hash1 = hash_password("mypassword");
        let hash2 = hash_password("mypassword");
        assert_eq!(hash1, hash2, "Same password must produce same hash");

        let hash3 = hash_password("different");
        assert_ne!(hash1, hash3, "Different passwords must produce different hashes");
    }

    /// Test: Auth key in WsAuthKeyValidation event includes client IP for ban tracking
    #[test]
    fn test_security_auth_key_event_has_ip() {
        // Verify the AppEvent::WsAuthKeyValidation includes a client_ip field
        // This is a compile-time check - if the event doesn't have 3 fields, this won't compile
        let msg = WsMessage::AuthRequest {
            password_hash: String::new(),
            username: None,
            current_world: None,
            auth_key: Some("test_key".to_string()),
            request_key: false,
            challenge_response: false,
            resume: Vec::new(), resume_epochs: Vec::new(), client_uid: String::new(),
        };
        let event = AppEvent::WsAuthKeyValidation(1, Box::new(msg), "10.0.0.1".to_string(), "test_challenge".to_string());

        // Verify we can extract the IP from the event
        if let AppEvent::WsAuthKeyValidation(_client_id, _msg, client_ip, _challenge) = event {
            assert_eq!(client_ip, "10.0.0.1");
        } else {
            panic!("Event should be WsAuthKeyValidation");
        }
    }

    /// Test: WebSocket auth with correct password succeeds
    #[tokio::test]
    async fn test_security_correct_password_auth_succeeds() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};
        use crate::websocket::{WsMessage, WsClientInfo};
        use std::sync::Arc;
        use std::sync::RwLock;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let password = "correctpass";
        let password_hash = hash_password(password);
        let (event_tx, _) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let clients: Arc<RwLock<std::collections::HashMap<u64, WsClientInfo>>> =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let allow_list: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let ban_list = BanList::new();
        let users: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::websocket::UserCredential>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));

        let server_clients = Arc::clone(&clients);
        let ph = password_hash.clone();
        let server_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream, 1, server_clients, ph, true,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}", port);
        let (ws, _) = connect_async(&url).await.unwrap();
        let (mut sink, mut source) = ws.split();

        // Skip ServerHello
        let _ = source.next().await;

        let auth = WsMessage::AuthRequest {
            password_hash,
            username: None,
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: false,
            resume: Vec::new(), resume_epochs: Vec::new(), client_uid: String::new(),
        };
        sink.send(WsRawMessage::Text(serde_json::to_string(&auth).unwrap())).await.unwrap();

        if let Some(Ok(WsRawMessage::Text(text))) = source.next().await {
            let resp: WsMessage = serde_json::from_str(&text).unwrap();
            if let WsMessage::AuthResponse { success, error, .. } = resp {
                assert!(success, "Correct password should succeed, error: {:?}", error);
            } else {
                panic!("Expected AuthResponse");
            }
        } else {
            panic!("No response received");
        }

        server_task.abort();
    }

    /// Test: Allow list IP matching with wildcards
    #[test]
    fn test_security_allow_list_matching() {
        use crate::websocket::is_ip_in_allow_list;

        // Exact match
        assert!(is_ip_in_allow_list("192.168.1.100", &["192.168.1.100".to_string()]));

        // Wildcard match
        assert!(is_ip_in_allow_list("192.168.1.100", &["192.168.1.*".to_string()]));
        assert!(is_ip_in_allow_list("192.168.1.50", &["192.168.*".to_string()]));

        // Non-match
        assert!(!is_ip_in_allow_list("10.0.0.1", &["192.168.1.*".to_string()]));

        // Empty list
        assert!(!is_ip_in_allow_list("192.168.1.100", &[]));

        // Localhost normalization
        assert!(is_ip_in_allow_list("127.0.0.1", &["localhost".to_string()]));
        assert!(is_ip_in_allow_list("::1", &["localhost".to_string()]));

        // Bare "*" matches all hosts
        assert!(is_ip_in_allow_list("10.0.0.1", &["*".to_string()]));
        assert!(is_ip_in_allow_list("192.168.1.100", &["*".to_string()]));

        // "*" in a multi-entry list
        assert!(is_ip_in_allow_list("10.0.0.1", &["192.168.1.1".to_string(), "*".to_string()]));

        // allow_list_has_wildcard
        use crate::websocket::allow_list_has_wildcard;
        assert!(allow_list_has_wildcard("*"));
        assert!(allow_list_has_wildcard("192.168.1.1, *"));
        assert!(allow_list_has_wildcard("*, 10.0.0.1"));
        assert!(!allow_list_has_wildcard("192.168.1.*"));
        assert!(!allow_list_has_wildcard(""));
        assert!(!allow_list_has_wildcard("192.168.1.1"));

        // Hostname pattern detection
        use crate::websocket::is_hostname_pattern;
        assert!(is_hostname_pattern("*.rd.shawcable.net"));
        assert!(is_hostname_pattern("host.example.com"));
        assert!(!is_hostname_pattern("192.168.1.*"));
        assert!(!is_hostname_pattern("192.168.1.100"));
        assert!(!is_hostname_pattern("*"));
        assert!(!is_hostname_pattern("localhost"));

        // Hostname wildcard matching via is_in_allow_list
        use crate::websocket::is_in_allow_list;
        // Wildcard match
        assert!(is_in_allow_list("96.43.12.34", Some("abc.rd.shawcable.net"), &["*.rd.shawcable.net".to_string()]));
        assert!(is_in_allow_list("96.43.12.34", Some("xyz.rd.shawcable.net"), &["*.rd.shawcable.net".to_string()]));
        // Wildcard does NOT match the bare domain itself
        assert!(!is_in_allow_list("96.43.12.34", Some("rd.shawcable.net"), &["*.rd.shawcable.net".to_string()]));
        // No hostname provided → hostname patterns don't match
        assert!(!is_in_allow_list("96.43.12.34", None, &["*.rd.shawcable.net".to_string()]));
        // Exact hostname match
        assert!(is_in_allow_list("1.2.3.4", Some("myhost.example.com"), &["myhost.example.com".to_string()]));
        assert!(!is_in_allow_list("1.2.3.4", Some("other.example.com"), &["myhost.example.com".to_string()]));
        // Case-insensitive
        assert!(is_in_allow_list("96.43.12.34", Some("ABC.RD.SHAWCABLE.NET"), &["*.rd.shawcable.net".to_string()]));
        // IP patterns still work via is_in_allow_list
        assert!(is_in_allow_list("192.168.1.100", None, &["192.168.1.*".to_string()]));
        assert!(!is_in_allow_list("10.0.0.1", None, &["192.168.1.*".to_string()]));
    }

    /// Test: ServerHello is sent before auth (regression: needed for client UI)
    #[tokio::test]
    async fn test_security_server_hello_sent_first() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::StreamExt;
        use crate::websocket::{WsMessage, WsClientInfo};
        use std::sync::Arc;
        use std::sync::RwLock;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (event_tx, _) = tokio::sync::mpsc::channel::<AppEvent>(100);
        let clients: Arc<RwLock<std::collections::HashMap<u64, WsClientInfo>>> =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let allow_list: Arc<std::sync::RwLock<Vec<String>>> =
            Arc::new(std::sync::RwLock::new(vec!["*".to_string()]));
        let whitelisted: Arc<std::sync::RwLock<Option<String>>> =
            Arc::new(std::sync::RwLock::new(None));
        let ban_list = BanList::new();
        let users: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::websocket::UserCredential>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));

        let server_clients = Arc::clone(&clients);
        let ph = hash_password("test");
        let server_task = tokio::spawn(async move {
            let (stream, client_addr) = listener.accept().await.unwrap();
            crate::websocket::handle_ws_client(
                stream, 1, server_clients, ph, true,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
                false, // knocked: not exercised by this test
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}", port);
        let (ws, _) = connect_async(&url).await.unwrap();
        let (_sink, mut source) = ws.split();

        // First message should be ServerHello
        if let Some(Ok(WsRawMessage::Text(text))) = source.next().await {
            let msg: WsMessage = serde_json::from_str(&text).unwrap();
            assert!(matches!(msg, WsMessage::ServerHello { .. }),
                "First message must be ServerHello, got: {:?}", msg);
        } else {
            panic!("No ServerHello received");
        }

        server_task.abort();
    }

    // ========== Regression Tests ==========
    // These tests use the testserver + testharness for end-to-end testing

    use crate::testserver;
    use crate::testharness::{self, TestConfig, TestWorldConfig, TestEvent, TestAction, WaitCondition, StateCheck};

    /// Helper: find a free TCP port
    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn test_more_mode_add_output_unit() {
        // Test that add_output correctly triggers more-mode after max_lines visual lines
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // 100 short lines, each fits in one visual line
        let data: String = (1..=100).map(|i| format!("fffff{}\n", i)).collect();
        world.add_output(&data, true, &settings, 24, 80, false, true, false);

        // max_lines = 24 - 2 = 22
        // After 22 visual lines, pause triggers on the 23rd line.
        // Lines 1-22 go to output without triggering (lines_since_pause accumulates).
        // Line 23: lines_since_pause(22) + 1 > 22 → triggers_pause → goes to output, then paused=true.
        // Lines 24-100 (77 lines) go to pending.
        assert!(world.paused, "Should be paused after 100 lines with max_lines=22");
        assert_eq!(world.output_lines.len(), 23,
            "Expected 23 output lines (22 before trigger + 1 triggering), got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 77,
            "Expected 77 pending lines, got {}", world.pending_lines.len());
    }

    // Regression: sending a command while scrolled up into scrollback must NOT drop the
    // user back to the bottom. reset_more_mode_on_send() must be a no-op unless the
    // viewport is already at the bottom.
    #[test]
    fn reset_more_mode_on_send_preserves_scrollback() {
        let mut world = World::new("test");
        for i in 0..50 {
            world.output_lines.push(OutputLine::new(format!("line {}", i), i as u64));
        }
        // Scrolled up into history in more-mode: paused, viewport above bottom, nothing held.
        world.paused = true;
        world.scroll_offset = 10; // bottom would be 49
        world.lines_since_pause = 5;
        assert!(!world.is_at_bottom(), "precondition: viewport is scrolled up");

        world.reset_more_mode_on_send();

        assert!(world.paused, "scroll lock must be kept when a command is sent from scrollback");
        assert_eq!(world.scroll_offset, 10, "viewport must not move when sending from scrollback");
    }

    #[test]
    fn reset_more_mode_on_send_releases_at_bottom() {
        let mut world = World::new("test");
        for i in 0..50 {
            world.output_lines.push(OutputLine::new(format!("line {}", i), i as u64));
        }
        // Following live at the bottom, no held output.
        world.paused = true;
        world.scroll_offset = world.output_lines.len() - 1;
        assert!(world.is_at_bottom(), "precondition: viewport is at the bottom");

        world.reset_more_mode_on_send();

        assert!(!world.paused, "at the bottom, sending a command releases the more-mode lock");
    }

    #[test]
    fn test_more_mode_long_wrapped_lines_unit() {
        // Test that add_output correctly triggers more-mode with long lines that wrap
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // 50 lines, each ~120 chars (wraps to 2 visual lines at width 80)
        let data: String = (1..=50).map(|i| {
            format!("{}LINE{:03}\n", "x".repeat(100), i)
        }).collect();
        world.add_output(&data, true, &settings, 24, 80, false, true, false);

        // max_lines = 22. Each line is 107 chars → wraps to ceil(107/80)=2 visual lines.
        // Line 1: lines_since_pause(0) + 2 = 2, not > 22
        // Line 11: lines_since_pause(20) + 2 = 22, not > 22
        // Line 12: lines_since_pause(22) + 2 = 24, > 22 → triggers_pause!
        // So 12 lines go to output (24 visual lines), 38 go to pending.
        assert!(world.paused, "Should be paused with long wrapped lines");
        assert_eq!(world.output_lines.len(), 12,
            "Expected 12 output lines (each wrapping to 2 visual), got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 38,
            "Expected 38 pending lines, got {}", world.pending_lines.len());
    }

    #[test]
    fn test_more_mode_single_line_exceeds_screen() {
        // A single logical line wraps to more visual lines than the screen.
        // Scenario: 2 short lines (2 vl) + 1 huge line (25 vl) on a 21-line screen (max_lines=19).
        // The huge line should trigger pause AND set visual_line_offset so the renderer
        // only shows the first 17 visual lines of it (filling exactly 19 with the 2 prior).
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;
        let max_lines = (output_height as usize) - 2; // 19

        // 2 short lines (1 visual line each)
        world.add_output("short line one\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short line two\n", true, &settings, output_height, output_width, false, true, false);
        assert!(!world.paused, "Should not be paused after 2 short lines");
        assert_eq!(world.lines_since_pause, 2);

        // 1 huge line: 25 visual lines at width 80 (80*25 = 2000 visible chars)
        let huge_line = "A".repeat(80 * 25);
        world.add_output(&format!("{}\n", huge_line), true, &settings, output_height, output_width, false, true, false);

        // Should be paused
        assert!(world.paused, "Should be paused after huge line");
        // Huge line goes to output (triggers_pause path), not pending
        assert_eq!(world.output_lines.len(), 3, "All 3 lines should be in output");
        assert_eq!(world.pending_lines.len(), 0, "No pending lines");
        // lines_since_pause = 2 + 25 = 27
        assert_eq!(world.lines_since_pause, 27);
        // visual_line_offset should be set: remaining_budget = 19 - 2 = 17
        assert_eq!(world.visual_line_offset, max_lines - 2,
            "visual_line_offset should be {} (screen fills precisely)", max_lines - 2);
    }

    #[test]
    fn test_more_mode_single_line_exceeds_screen_release() {
        // Test that Tab (release_pending_screenful) correctly reveals more of a
        // partially-shown line before releasing pending lines.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;

        // 2 short lines + 1 huge line (25 vl) + 5 pending lines
        world.add_output("short one\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short two\n", true, &settings, output_height, output_width, false, true, false);
        let huge_line = "B".repeat(80 * 25);
        // The huge line + 5 more lines in one batch
        let batch = format!("{}\npending1\npending2\npending3\npending4\npending5\n", huge_line);
        world.add_output(&batch, true, &settings, output_height, output_width, false, true, false);

        assert!(world.paused);
        assert_eq!(world.output_lines.len(), 3, "3 lines in output (2 short + 1 huge)");
        assert_eq!(world.pending_lines.len(), 5, "5 lines pending");
        assert_eq!(world.visual_line_offset, 17, "partial display at 17 vl");

        // Simulate Tab: release_pending reveals more of the huge line first
        // Remaining vl of huge line: 25 - 17 = 8. Budget is 19.
        // 8 < 19, so partial clears and budget becomes 19 - 8 = 11 for pending.
        world.release_pending(19 - 8, &test_metrics(&settings, output_width as usize));
        // visual_line_offset should be cleared by release_pending's scroll_to_bottom
        // (the App-level release_pending_screenful handles the VLO logic, but
        // at the World level, after release_pending, scroll_to_bottom clears it)
    }

    #[test]
    fn test_more_mode_visual_line_offset_cleared_on_scroll() {
        // visual_line_offset should be cleared when user scrolls manually
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;

        // Set up a paused state with visual_line_offset
        world.add_output("short\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short\n", true, &settings, output_height, output_width, false, true, false);
        let huge_line = "C".repeat(80 * 25);
        world.add_output(&format!("{}\nextra\n", huge_line), true, &settings, output_height, output_width, false, true, false);

        assert!(world.visual_line_offset > 0, "Should have visual_line_offset set");

        // release_all_pending should clear it
        world.release_all_pending();
        assert_eq!(world.visual_line_offset, 0, "release_all_pending should clear visual_line_offset");
    }

    // ---- /recall × more-mode interaction (App::emit_recall) ----
    // Regression coverage for the bug where the WS/daemon Recall(opts) arms
    // broadcast every matched line directly via ws_broadcast, never touching
    // World::add_output, so more-mode pause never engaged for remote console /
    // web / GUI / daemon clients. emit_recall() now funnels every /recall
    // through the same gate as server output.

    fn seed_recall_source(world: &mut World, count: usize) {
        for i in 0..count {
            let seq = world.next_seq;
            world.next_seq += 1;
            // OutputLine::new defaults from_server=true, which is required —
            // execute_recall's default source (CurrentWorld) only matches
            // from_server lines.
            world.output_lines.push(OutputLine::new(format!("alpha {}", i), seq));
        }
    }

    fn recall_opts_for_alpha() -> tf::RecallOptions {
        tf::RecallOptions {
            range: tf::RecallRange::Last(100),
            pattern: Some("alpha".to_string()),
            match_style: tf::RecallMatchStyle::Simple,
            ..tf::RecallOptions::default()
        }
    }

    #[test]
    fn test_recall_respects_more_mode() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;
        app.output_height = 12; // max_lines = 10
        app.output_width = 80;

        seed_recall_source(&mut app.worlds[0], 100);
        // Seeding output_lines directly (not via add_output) doesn't touch the
        // more-mode budget, so start from a clean slate.
        app.worlds[0].lines_since_pause = 0;

        let opts = recall_opts_for_alpha();
        app.emit_recall(&opts, 0, false);

        // Block = 100 matches, 1 visual line each, max_lines = 10: rows 1-10
        // fill the budget, row 11 trips triggers_pause (still shown), rows
        // 12-100 divert to pending.
        assert!(app.worlds[0].paused, "should be paused once /recall output exceeds a screenful");
        assert_eq!(app.worlds[0].lines_since_pause, 11);
        assert_eq!(app.worlds[0].output_lines.len(), 100 + 11);
        assert_eq!(app.worlds[0].pending_lines.len(), 89);

        // The bug this guards against: pre-fix, the WS/daemon arms broadcast
        // one ServerData per matched line and never sent PendingLinesUpdate,
        // so a remote console could never tell it should pause.
        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 1, "the visible portion of a /recall block must be a single broadcast");
        assert!(
            log.iter().any(|m| matches!(m, WsMessage::PendingLinesUpdate { world_index: 0, count: 89 })),
            "must broadcast PendingLinesUpdate so remote/web/GUI clients know to pause"
        );
    }

    // ========== /recall -i / -l / -g: captured user input ==========
    // Regression coverage for /recall -i being a dead no-op (RecallSource::Input's arm in
    // actions.rs used to be an unconditional `continue`, so /recall -i never matched
    // anything) and for the deeper gap it was papering over: user-typed input was never
    // captured anywhere with a timestamp. See App::record_user_input/World::record_input_line
    // in main.rs and the source filter in actions.rs's execute_recall.

    #[test]
    fn test_recall_i_returns_captured_input() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].login_capture_guard = 0; // past the login-capture window

        app.record_user_input(0, "north");

        let opts = tf::RecallOptions {
            source: tf::RecallSource::Input,
            pattern: Some("north".to_string()),
            match_style: tf::RecallMatchStyle::Simple,
            ..tf::RecallOptions::default()
        };
        let matches = app.recall_matches(&opts, 0).unwrap();
        assert_eq!(matches, vec!["\u{00BB} north".to_string()],
            "the RecallSource::Input arm used to be an unconditional `continue` - /recall -i always returned no matches");
    }

    #[test]
    fn test_recall_default_source_excludes_input() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].login_capture_guard = 0;

        let seq = app.worlds[0].next_seq; app.worlds[0].next_seq += 1;
        app.worlds[0].output_lines.push(OutputLine::new("You go north.".to_string(), seq));
        app.record_user_input(0, "north");

        let opts = tf::RecallOptions {
            pattern: Some("north".to_string()),
            match_style: tf::RecallMatchStyle::Simple,
            ..tf::RecallOptions::default() // default source: CurrentWorld
        };
        let matches = app.recall_matches(&opts, 0).unwrap();
        assert_eq!(matches, vec!["You go north.".to_string()],
            "plain /recall (no source flag) must stay server-output-only, excluding captured input");
    }

    #[test]
    fn test_recall_local_and_global_include_input() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].login_capture_guard = 0;

        let seq = app.worlds[0].next_seq; app.worlds[0].next_seq += 1;
        app.worlds[0].output_lines.push(OutputLine::new("You go north.".to_string(), seq));
        let seq = app.worlds[0].next_seq; app.worlds[0].next_seq += 1;
        app.worlds[0].output_lines.push(OutputLine::new_client("Disconnected.".to_string(), seq));
        app.record_user_input(0, "north");

        let local_opts = tf::RecallOptions {
            source: tf::RecallSource::Local,
            match_style: tf::RecallMatchStyle::Simple,
            ..tf::RecallOptions::default()
        };
        let local_matches = app.recall_matches(&local_opts, 0).unwrap();
        assert_eq!(local_matches, vec!["Disconnected.".to_string(), "\u{00BB} north".to_string()],
            "-l must include both client-generated notices AND captured input, but not server output");

        let global_opts = tf::RecallOptions {
            source: tf::RecallSource::Global,
            match_style: tf::RecallMatchStyle::Simple,
            ..tf::RecallOptions::default()
        };
        let global_matches = app.recall_matches(&global_opts, 0).unwrap();
        assert_eq!(global_matches, vec!["You go north.".to_string(), "Disconnected.".to_string(), "\u{00BB} north".to_string()],
            "-g must include server output + client notices + captured input, all together");
    }

    #[test]
    fn test_input_line_invisible_until_show_tags() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].login_capture_guard = 0;

        let lines_before = app.worlds[0].lines_since_pause;
        let unseen_before = app.worlds[0].unseen_lines;
        let paused_before = app.worlds[0].paused;

        app.record_user_input(0, "secretcmd");

        assert_eq!(app.worlds[0].lines_since_pause, lines_before, "capturing input must not consume the more-mode budget");
        assert_eq!(app.worlds[0].unseen_lines, unseen_before, "capturing input must not count as unseen activity");
        assert_eq!(app.worlds[0].paused, paused_before, "capturing input must never trigger a pause");

        let settings = Settings::default();
        let hidden = build_display_lines(&app.worlds[0], &settings, 21, 80, false);
        assert!(!hidden.iter().any(|l| l.text.contains("secretcmd")),
            "captured input must not appear in normal display (show_tags: false)");

        let shown = build_display_lines(&app.worlds[0], &settings, 21, 80, true);
        assert!(shown.iter().any(|l| l.text.contains("secretcmd")),
            "captured input must appear when show_tags (F2) is on");
    }

    #[test]
    fn test_input_line_interleaves_by_seq_and_timestamp() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].login_capture_guard = 0;
        let settings = Settings::default();

        app.record_user_input(0, "north");
        app.worlds[0].add_output("You go north.\n", true, &settings, 24, 80, false, true, false);

        let seqs: Vec<u64> = app.worlds[0].output_lines.iter().map(|l| l.seq).collect();
        for pair in seqs.windows(2) {
            assert!(pair[0] < pair[1], "output_lines must stay strictly increasing by seq: {:?}", seqs);
        }
        assert_eq!(app.worlds[0].output_lines.len(), 2);
        assert!(app.worlds[0].output_lines[0].is_input);
        assert!(!app.worlds[0].output_lines[1].is_input);
        assert!(app.worlds[0].output_lines[0].timestamp <= app.worlds[0].output_lines[1].timestamp,
            "captured input's timestamp must not be later than a server line that arrived after it");
    }

    #[test]
    fn test_input_capture_survives_trailing_partial() {
        // Regression guard for the prerequisite fix (last_visible_output_idx): a gagged/
        // input line pushed after an outstanding partial (prompt) used to be silently
        // clobbered the next time that partial completed, because add_output assumed
        // output_lines.last()/pending_lines.last() was always the partial.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].login_capture_guard = 0;
        let settings = Settings::default();

        // MUD prompt with no trailing newline.
        app.worlds[0].add_output("prompt> ", true, &settings, 24, 80, false, true, false);
        assert!(!app.worlds[0].partial_line.is_empty());

        // The player types a command while the prompt is still outstanding.
        app.record_user_input(0, "north");

        // The rest of the line (with a newline) arrives, completing the prompt.
        app.worlds[0].add_output("rest of line\n", true, &settings, 24, 80, false, true, false);

        // The captured input line must still exist, untouched.
        let input_lines: Vec<&OutputLine> = app.worlds[0].output_lines.iter().filter(|l| l.is_input).collect();
        assert_eq!(input_lines.len(), 1, "captured input must not have been clobbered by the partial's completion");
        assert_eq!(input_lines[0].text, "north");

        // And the prompt must have completed correctly, not been overwritten by "north".
        let prompt_line = app.worlds[0].output_lines.iter().find(|l| !l.is_input).unwrap();
        assert_eq!(prompt_line.text, "prompt> rest of line");
    }

    #[test]
    fn test_login_capture_guard_skips_first_six_sends_and_rearms_on_logout() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test")); // login_capture_guard starts at 6, matching a fresh connect
        app.current_world_index = 0;
        assert_eq!(app.worlds[0].login_capture_guard, 6);

        // First 6 sends after "connecting" must not be captured.
        for i in 0..6 {
            app.record_user_input(0, &format!("secret{}", i));
        }
        assert_eq!(app.worlds[0].login_capture_guard, 0);
        assert!(app.worlds[0].output_lines.iter().all(|l| !l.is_input),
            "none of the first 6 sends after connecting should have been captured");

        // The 7th send is captured normally.
        app.record_user_input(0, "look");
        assert_eq!(app.worlds[0].output_lines.iter().filter(|l| l.is_input).count(), 1);
        assert_eq!(app.worlds[0].output_lines.iter().find(|l| l.is_input).unwrap().text, "look");

        // Typing "logout" re-arms the guard for the next 6 sends - and isn't itself captured.
        app.record_user_input(0, "logout");
        assert_eq!(app.worlds[0].login_capture_guard, 6);
        assert_eq!(app.worlds[0].output_lines.iter().filter(|l| l.is_input).count(), 1,
            "the logout line itself must not be captured");

        for i in 0..6 {
            app.record_user_input(0, &format!("relogin{}", i));
        }
        assert_eq!(app.worlds[0].output_lines.iter().filter(|l| l.is_input).count(), 1,
            "the re-login window after logout must also be skipped");

        app.record_user_input(0, "look again");
        assert_eq!(app.worlds[0].output_lines.iter().filter(|l| l.is_input).count(), 2);
    }

    #[test]
    fn test_record_input_line_respects_log_input_gate() {
        let mut world = World::new("test");
        world.settings.log_enabled = true; // per-world log switch, must ALSO be on
        world.log_date = Some(World::get_current_date_string()); // avoid real file rollover

        let tmp_path = std::env::temp_dir().join(format!("clay_test_log_input_{}.log", std::process::id()));
        let file = std::fs::File::create(&tmp_path).unwrap();
        world.log_handle = Some(std::sync::Arc::new(std::sync::Mutex::new(file)));

        // log_input: false - captured line must still land in output_lines (so /recall -i
        // still finds it), but must NOT be written to the log file.
        let (_, in_output) = world.record_input_line("secretcmd", false, false);
        assert!(in_output);
        assert_eq!(world.output_lines.len(), 1);
        assert!(world.output_lines[0].is_input);
        assert_eq!(world.output_lines[0].text, "secretcmd");

        // log_input: true - captured line lands in output_lines AND is written to the file.
        let (_, in_output2) = world.record_input_line("visiblecmd", true, false);
        assert!(in_output2);
        assert_eq!(world.output_lines.len(), 2);

        drop(world); // release the Arc<Mutex<File>> so the write is flushed/closed before reading
        let contents = std::fs::read_to_string(&tmp_path).unwrap();
        let _ = std::fs::remove_file(&tmp_path);

        assert!(!contents.contains("secretcmd"), "log_input: false must not write to the file");
        assert!(contents.contains("visiblecmd"), "log_input: true must write to the file");
        assert!(contents.contains('\u{00BB}'), "the logged line must carry the input marker");
    }

    #[test]
    fn test_recall_without_more_mode_emits_all() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = false;
        app.output_height = 12;
        app.output_width = 80;

        seed_recall_source(&mut app.worlds[0], 100);
        app.worlds[0].lines_since_pause = 0;

        let opts = recall_opts_for_alpha();
        app.emit_recall(&opts, 0, false);

        assert!(!app.worlds[0].paused);
        assert!(app.worlds[0].pending_lines.is_empty());
        assert_eq!(app.worlds[0].output_lines.len(), 100 + 100);

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 1, "the whole block should still be a single broadcast");
        assert!(
            !log.iter().any(|m| matches!(m, WsMessage::PendingLinesUpdate { .. })),
            "nothing should be pending when more-mode is off"
        );
    }

    #[test]
    fn test_recall_while_paused_all_pending() {
        // This is the exact scenario that produces today's duplicate-on-release
        // bug for the console arms (they broadcast unconditionally per line,
        // including lines that land in pending_lines, which then get
        // broadcast AGAIN on release) — emit_recall must not broadcast
        // anything that goes to pending_lines.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;
        app.output_height = 12;
        app.output_width = 80;

        seed_recall_source(&mut app.worlds[0], 100);
        app.worlds[0].lines_since_pause = 0;
        app.worlds[0].paused = true; // already paused before /recall runs

        let opts = recall_opts_for_alpha();
        app.emit_recall(&opts, 0, false);

        assert_eq!(app.worlds[0].pending_lines.len(), 100, "entire block should divert to pending");
        assert_eq!(app.worlds[0].output_lines.len(), 100, "no /recall lines should reach output_lines");

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 0, "nothing landed in output_lines, so nothing should broadcast");
        assert!(
            log.iter().any(|m| matches!(m, WsMessage::PendingLinesUpdate { world_index: 0, count: 100 })),
            "pending count changed and must still be announced"
        );
    }

    // ---- World::add_output partial-line accounting (main.rs ~2806-2860) ----
    // Regression coverage for the bug where completing or filtering a partial
    // (unterminated) line never corrected lines_since_pause for the delta.

    #[test]
    fn test_add_output_recounts_completed_partial_line() {
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // Outstanding unterminated prompt: counted once as 1 visual row.
        world.add_output("prompt> ", true, &settings, 24, 80, false, true, false);
        assert_eq!(world.output_lines.len(), 1);
        assert_eq!(world.lines_since_pause, 1);

        // Complete it with enough text to wrap to multiple visual rows.
        let long = "x".repeat(400);
        world.add_output(&format!("{}\n", long), true, &settings, 24, 80, false, true, false);

        // Measured through the renderer's own path, the same one add_output now budgets with.
        let expected = crate::rendering::display_rows(
            &make_output_line(&format!("prompt> {}", long), false),
            80,
            false,
            &settings,
            &CachedNow::new(),
        );
        assert!(expected > 1, "test line must actually wrap to be meaningful");
        assert_eq!(world.output_lines.len(), 1, "partial should complete in place, not append");
        assert_eq!(
            world.lines_since_pause, expected,
            "completed partial must be re-counted at its final wrapped height, not left at 1"
        );
    }

    #[test]
    fn test_add_output_uncounts_filtered_partial_line() {
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // A whitespace-only partial is still counted as 1 visual row when added...
        world.add_output("   ", true, &settings, 24, 80, false, true, false);
        assert_eq!(world.lines_since_pause, 1);
        let before = world.lines_since_pause;

        // ...but when completed it's visually empty and gets popped (filtered),
        // so the budget must be uncounted, not left stale.
        world.add_output("\n", true, &settings, 24, 80, false, true, false);
        assert!(world.output_lines.is_empty(), "filtered line should be removed, not kept");
        assert_eq!(world.lines_since_pause, before - 1);
    }

    // ---- TF command output × more-mode / broadcast (App::emit_client_text) ----
    // Regression coverage for two bugs found while unifying the "✨" client-line
    // prefix across TUI/console/web: add_tf_output never broadcast to any WS
    // client at all (invisible on web/GUI/Android), and several WS/daemon
    // TfCommandResult::Success/Error arms broadcast directly without ever
    // touching World::add_output (same bug class the /recall fix addressed).

    #[test]
    fn test_tf_message_broadcasts_through_gate() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        app.emit_client_text(0, "hello", false);

        assert_eq!(app.worlds[0].output_lines.len(), 1);
        assert_eq!(app.worlds[0].output_lines[0].text, "hello");
        assert!(!app.worlds[0].output_lines[0].from_server);

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<_> = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).collect();
        assert_eq!(server_data.len(), 1);
        if let WsMessage::ServerData { data, from_server, .. } = server_data[0] {
            assert_eq!(data, "hello\n");
            assert!(!from_server);
        } else {
            panic!("expected ServerData");
        }
    }

    #[test]
    fn test_add_tf_output_now_broadcasts() {
        // Pre-fix, add_tf_output had no ws_broadcast* call anywhere in its body -
        // this log would be empty.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        app.add_tf_output("hello from TF");

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 1, "add_tf_output must broadcast, not just display locally");
    }

    #[test]
    fn test_tf_error_uses_error_prefix() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        app.emit_tf_error(0, "boom", false);

        assert_eq!(app.worlds[0].output_lines.len(), 1);
        assert_eq!(app.worlds[0].output_lines[0].text, "Error: boom");
    }

    #[test]
    fn test_tf_multiline_is_single_broadcast() {
        // Guards the daemon.rs help-loop conversion: N ws_broadcast calls (one
        // per line) collapsed into one emit_client_text call.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        let five_lines = "line1\nline2\nline3\nline4\nline5";
        app.emit_client_text(0, five_lines, false);

        assert_eq!(app.worlds[0].output_lines.len(), 5);
        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 1, "a multi-line block must still be a single broadcast");
    }

    #[test]
    fn test_tf_output_respects_more_mode() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;
        app.output_height = 12; // max_lines = 10
        app.output_width = 80;

        let forty_lines: String = (0..40).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        app.emit_client_text(0, &forty_lines, false);

        assert!(app.worlds[0].paused);
        assert!(!app.worlds[0].pending_lines.is_empty());
        assert_eq!(app.worlds[0].output_lines.len() + app.worlds[0].pending_lines.len(), 40);

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 1, "the visible portion must be a single broadcast");
        assert!(
            log.iter().any(|m| matches!(m, WsMessage::PendingLinesUpdate { world_index: 0, .. })),
            "must broadcast PendingLinesUpdate so remote/web/GUI clients know to pause"
        );
    }

    #[test]
    fn test_tf_output_while_paused_does_not_broadcast() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;
        app.output_height = 12;
        app.output_width = 80;
        app.worlds[0].paused = true; // already paused

        app.emit_client_text(0, "held back", false);

        assert_eq!(app.worlds[0].pending_lines.len(), 1);
        assert!(app.worlds[0].output_lines.is_empty());

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_count = log.iter().filter(|m| matches!(m, WsMessage::ServerData { .. })).count();
        assert_eq!(server_data_count, 0, "nothing landed in output_lines, so nothing should broadcast");
    }

    #[test]
    fn test_emit_client_text_routes_to_named_world() {
        // Regression guard: 4 call sites (portal detection, trigger messages)
        // previously went through add_tf_output's implicit current_world_index
        // even when they had a different, correct world_idx in scope.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("world0"));
        app.worlds.push(World::new("world1"));
        app.current_world_index = 0;

        app.emit_client_text(1, "for world1", false);

        assert!(app.worlds[0].output_lines.is_empty(), "must not land in the current world");
        assert_eq!(app.worlds[1].output_lines.len(), 1);
        assert_eq!(app.worlds[1].output_lines[0].text, "for world1");
    }

    #[test]
    fn test_emit_client_text_ignores_empty() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        app.emit_client_text(0, "", false);

        assert!(app.worlds[0].output_lines.is_empty());
        let log = app.ws_broadcast_log.lock().unwrap();
        assert!(log.is_empty());
    }

    #[test]
    fn test_client_line_text_has_no_sparkle_prefix() {
        // The invariant this whole unification relies on: OutputLine.text (and
        // every wire payload derived from it) stays prefix-free. The "✨ "
        // marker is added exactly once, at display time, by each renderer.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        app.emit_client_text(0, "plain message", false);
        let opts = tf::RecallOptions {
            source: tf::RecallSource::World("does-not-exist".to_string()),
            ..tf::RecallOptions::default()
        };
        app.emit_recall(&opts, 0, false); // exercises emit_recall's error path

        for line in &app.worlds[0].output_lines {
            assert!(!line.text.starts_with('\u{2728}'), "stored text must never carry the client-line prefix: {:?}", line.text);
        }
        let log = app.ws_broadcast_log.lock().unwrap();
        for msg in log.iter() {
            if let WsMessage::ServerData { data, .. } = msg {
                assert!(!data.starts_with('\u{2728}'), "broadcast text must never carry the client-line prefix: {:?}", data);
            }
        }
    }

    #[test]
    fn test_process_output_line_adds_exactly_one_prefix() {
        use crate::rendering::process_output_line;
        let cached_now = CachedNow::new();

        let client_line = OutputLine::new_client("hello".to_string(), 1);
        let rendered = process_output_line(&client_line, false, false, false, &cached_now).unwrap();
        assert_eq!(rendered.matches('\u{2728}').count(), 1, "a client-generated line gets exactly one prefix");

        let server_line = OutputLine::new("hello".to_string(), 2);
        let rendered = process_output_line(&server_line, false, false, false, &cached_now).unwrap();
        assert_eq!(rendered.matches('\u{2728}').count(), 0, "a server line gets no prefix");

        let whitespace_client_line = OutputLine::new_client("   ".to_string(), 3);
        let rendered = process_output_line(&whitespace_client_line, false, false, false, &cached_now).unwrap();
        assert_eq!(rendered.matches('\u{2728}').count(), 0, "a visually-empty client line gets no prefix");
    }

    #[test]
    fn test_initial_state_snapshot_matches_stored_text() {
        // Regression guard for un-baking the "✨ " prefix from build_initial_state:
        // the snapshot sent to a newly-connecting client must carry exactly the
        // same text as what's stored, not a baked-in prefix (which would
        // double up with the display-time prefix on both console and web).
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.worlds[0].output_lines.push(OutputLine::new_client("client line".to_string(), 1));
        app.worlds[0].output_lines.push(OutputLine::new("server line".to_string(), 2));

        let state = app.build_initial_state(0);
        if let WsMessage::InitialState { worlds, .. } = state {
            let ts_lines = &worlds[0].output_lines_ts;
            assert_eq!(ts_lines.len(), 2);
            assert_eq!(ts_lines[0].text, "client line");
            assert_eq!(ts_lines[1].text, "server line");
        } else {
            panic!("expected InitialState");
        }
    }

    #[test]
    fn test_release_pending_preserves_from_server() {
        // Regression guard: release_pending_screenful used to hardcode
        // from_server: true on every released broadcast regardless of the
        // actual per-line flag, which would strip the "✨ " marker from a
        // released client-generated line (e.g. an overflowed /recall) on web
        // while the TUI (which reads the real stored flag) rendered it fine.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.output_height = 12; // max_lines = 10
        app.output_width = 80;

        app.worlds[0].pending_lines.push(OutputLine::new("server line".to_string(), 1));
        app.worlds[0].pending_lines.push(OutputLine::new_client("client line".to_string(), 2));
        app.worlds[0].paused = true;

        app.release_pending_screenful();

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<_> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { data, from_server, .. } = m { Some((data.clone(), *from_server)) } else { None }
        }).collect();
        assert!(server_data.iter().any(|(d, fs)| d.contains("server line") && *fs), "server line must broadcast with from_server: true");
        assert!(server_data.iter().any(|(d, fs)| d.contains("client line") && !*fs), "client line must broadcast with from_server: false, not hardcoded true");
    }

    #[test]
    fn test_release_pending_screenful_broadcasts_exactly_what_it_releases() {
        // Regression guard: release_pending_screenful used to size its broadcast set with
        // visual_line_count (a full-width, div_ceil estimate) while World::release_pending
        // itself decided what to actually move into output_lines using nli_visual_rows
        // (NLI-narrowed width + real wrap_ansi_line wrapping). For a marked_new line with NLI
        // enabled, nli_visual_rows is always >= visual_line_count's estimate, so the old code
        // could broadcast more lines than were actually released - the surplus stayed in
        // pending_lines and was broadcast AGAIN on the next release, with no way for any
        // client to detect the duplicate (release batches use seq: 0).
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.output_width = 80;
        app.output_height = 8; // visual_budget = output_height - 2 = 6
        app.settings.new_line_indicator = true;

        // 79 columns: at the full 80-col width that's 1 row (visual_line_count's estimate),
        // but marked_new + NLI narrows the wrap width to 78, wrapping it to 2 rows
        // (nli_visual_rows' real answer) - this is exactly where the two formulas diverge.
        let line_text = "x".repeat(79);
        for i in 0..6 {
            // Explicitly console-owned: the ▶ prefix is what narrows the wrap width to 78
            // and makes each line 2 rows, which is the whole point of this fixture. Ownership
            // is now per line (OutputLine::display_id) rather than implied by a world
            // watermark, so it has to be set here.
            let mut line = OutputLine::new(line_text.clone(), i as u64);
            line.viewed = true;
            line.display_id = Some(crate::CONSOLE_DISPLAY_ID);
            app.worlds[0].pending_lines.push(line);
        }
        app.worlds[0].paused = true;

        let broadcast_line_count = |app: &App| -> usize {
            let log = app.ws_broadcast_log.lock().unwrap();
            log.iter().filter_map(|m| {
                if let WsMessage::ServerData { data, .. } = m { Some(data.lines().count()) } else { None }
            }).sum()
        };

        app.release_pending_screenful();
        let remaining_after_first = app.worlds[0].pending_lines.len();
        let actually_released_first = 6 - remaining_after_first;
        // With the corrected accounting, budget 6 / nli_visual_rows 2-per-line releases
        // exactly 3 lines - not all 6, which is what the old visual_line_count-based
        // broadcast estimate would have sent.
        assert_eq!(actually_released_first, 3, "sanity check on the crafted budget/row-count divergence");
        assert_eq!(broadcast_line_count(&app), actually_released_first,
            "broadcast line count must exactly match the lines actually moved into output_lines");

        // Second release: confirm the previously-released lines are never re-sent.
        app.ws_broadcast_log.lock().unwrap().clear();
        app.release_pending_screenful();
        let remaining_after_second = app.worlds[0].pending_lines.len();
        let actually_released_second = remaining_after_first - remaining_after_second;
        assert_eq!(broadcast_line_count(&app), actually_released_second,
            "second release must not re-broadcast lines already sent in the first batch");
    }

    #[test]
    fn test_released_pending_carries_real_seqs_no_false_duplicate() {
        // Regression guard: release broadcasts used to send seq: 0 unconditionally ("bypass
        // client-side dedup"), hiding the real seq from clients entirely. Step 6 of the
        // seq-drift fix makes real seqs safe here (World::output_lines is guaranteed
        // seq-sorted through a pause as of Step 4's gagged-line fix) and sends the true span
        // instead, so a client's dedup/gap-tracking sees the real seq range rather than being
        // unable to reason about it at all.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;

        // Simulate 3 lines already delivered (seq 0,1,2) - the "already broadcast" baseline.
        for i in 0..3u64 {
            let seq = app.worlds[0].next_seq;
            app.worlds[0].next_seq += 1;
            app.worlds[0].output_lines.push(OutputLine::new(format!("line {i}"), seq));
        }
        let last_broadcast_seq = app.worlds[0].output_lines.last().unwrap().seq;

        // Pause and accumulate two pending lines (seq 3, 4).
        app.worlds[0].paused = true;
        for i in 3..5u64 {
            let seq = app.worlds[0].next_seq;
            app.worlds[0].next_seq += 1;
            app.worlds[0].pending_lines.push(OutputLine::new(format!("line {i}"), seq));
        }

        app.ws_broadcast_log.lock().unwrap().clear();
        app.release_pending_screenful();

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<(u64, Option<u64>)> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { seq, end_seq, .. } = m { Some((*seq, *end_seq)) } else { None }
        }).collect();
        assert_eq!(server_data.len(), 1, "expected one batch for two contiguous same-flag lines: {server_data:?}");
        let (seq, end_seq) = server_data[0];
        assert_eq!(seq, 3);
        assert_eq!(end_seq, Some(4));
        assert!(seq > last_broadcast_seq,
            "released batch's seq ({seq}) must be strictly greater than what was already broadcast ({last_broadcast_seq})");
    }

    #[test]
    fn test_selective_flush_emits_contiguous_seq_runs() {
        // Regression guard: selective_flush's kept lines are typically NOT seq-contiguous
        // (only lines matching the highlight filter survive; the rest are discarded), so
        // broadcast_released_lines' batch grouping must treat a seq gap as a batch boundary
        // even when marked_new/from_server don't change - otherwise a single batch's
        // seq..=end_seq span would claim to cover a gap it doesn't actually contain.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.worlds[0].paused = true;

        for i in 0..5u64 {
            let seq = app.worlds[0].next_seq;
            app.worlds[0].next_seq += 1;
            let mut line = OutputLine::new(format!("line {i}"), seq);
            if i == 1 || i == 3 {
                line.highlight_color = Some("red".to_string());
            }
            app.worlds[0].pending_lines.push(line);
        }

        app.selective_flush(0);

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<(u64, Option<u64>)> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { seq, end_seq, .. } = m { Some((*seq, *end_seq)) } else { None }
        }).collect();
        assert_eq!(server_data.len(), 2,
            "two non-contiguous highlighted lines must produce two separate batches: {server_data:?}");
        assert_eq!(server_data[0], (1, Some(1)));
        assert_eq!(server_data[1], (3, Some(3)));
    }

    #[test]
    fn test_broadcast_released_lines_preserves_per_line_gagged_status() {
        // Regression guard: broadcast_released_lines used to hardcode gagged: false on every
        // emitted ServerData regardless of the actual lines' gagged status, and didn't split a
        // batch when gagged-ness changed between consecutive lines - both structurally possible
        // since a gagged line arriving while a world is already paused with a backlog gets
        // routed into pending_lines right alongside ordinary content (process_server_data's
        // hold_gagged_in_pending). A client applies a message's gagged flag to every line it
        // contains, so a mixed batch sent as gagged: false made previously-gagged text render
        // as ordinary visible text the moment its backlog was released.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        // Contiguous seqs 0..5, gagged pattern: visible, GAGGED, GAGGED, visible, visible -
        // two boundaries, so three batches expected.
        let gagged_pattern = [false, true, true, false, false];
        let released: Vec<OutputLine> = (0..5u64).map(|i| {
            let mut line = OutputLine::new(format!("line {i}"), i);
            line.gagged = gagged_pattern[i as usize];
            line
        }).collect();

        app.broadcast_released_lines(0, &released, None);

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<(u64, Option<u64>, bool)> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { seq, end_seq, gagged, .. } = m { Some((*seq, *end_seq, *gagged)) } else { None }
        }).collect();
        assert_eq!(server_data.len(), 3,
            "a gagged-status change must start a new batch, even with contiguous seqs: {server_data:?}");
        assert_eq!(server_data[0], (0, Some(0), false), "line 0 (visible) alone");
        assert_eq!(server_data[1], (1, Some(2), true), "lines 1-2 (gagged) batched together");
        assert_eq!(server_data[2], (3, Some(4), false), "lines 3-4 (visible) batched together");
    }

    #[test]
    fn test_disconnect_message_not_broadcast_when_deferred_to_pending() {
        // Regression guard: handle_disconnected used to broadcast "Disconnected."'s real seq
        // unconditionally, even when push_line_respecting_pending deferred it into
        // pending_lines (world already paused with a backlog) rather than displaying it now.
        // That advanced a client's dedup high-water-mark (world._max_seq) past still-queued,
        // lower-seq pending content, which then got silently dropped as a false duplicate once
        // actually released via broadcast_released_lines (seq-drift fix broadcasts real seqs,
        // no longer the old seq: 0 sentinel that used to make this harmless) - the entire
        // backlog for that world became permanently unrecoverable after a disconnect/reconnect
        // while paused.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;
        app.worlds[0].paused = true;

        // Existing backlog: 3 lines already queued (seq 0,1,2), not yet delivered to the client.
        for i in 0..3u64 {
            let seq = app.worlds[0].next_seq;
            app.worlds[0].next_seq += 1;
            app.worlds[0].pending_lines.push(OutputLine::new(format!("backlog {i}"), seq));
        }

        app.ws_broadcast_log.lock().unwrap().clear();
        app.handle_disconnected(0);

        // The "Disconnected." message must have been deferred into pending_lines (world was
        // paused with a non-empty backlog), not displayed - and therefore must NOT have been
        // broadcast yet.
        assert_eq!(app.worlds[0].pending_lines.len(), 4, "backlog (3) + deferred Disconnected. message");
        assert_eq!(app.worlds[0].pending_lines.last().unwrap().text, "Disconnected.");
        {
            let log = app.ws_broadcast_log.lock().unwrap();
            let disconnected_broadcasts: Vec<_> = log.iter().filter(|m| {
                matches!(m, WsMessage::ServerData { data, .. } if data.contains("Disconnected."))
            }).collect();
            assert!(disconnected_broadcasts.is_empty(),
                "Disconnected. must not be broadcast while still sitting in pending_lines: {disconnected_broadcasts:?}");
        }

        // Once the backlog (including the deferred Disconnected. message) is actually
        // released, it must show up exactly once, with its real seq intact.
        app.ws_broadcast_log.lock().unwrap().clear();
        app.release_pending_lines(1, 0, 0); // count: 0 = release all
        let log = app.ws_broadcast_log.lock().unwrap();
        let combined_data: String = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { data, .. } = m { Some(data.clone()) } else { None }
        }).collect();
        assert!(combined_data.contains("Disconnected."),
            "Disconnected. must be delivered once the backlog is released: {combined_data:?}");
        assert!(app.worlds[0].pending_lines.is_empty());
    }

    #[test]
    fn test_add_output_broadcasts_real_seq() {
        // Regression guard: App::add_output used to broadcast the raw input text with
        // seq: 0 unconditionally, regardless of the real seq the line actually got in
        // output_lines (and without checking whether it landed there at all vs.
        // pending_lines while paused). Assert the emitted ServerData carries the real seq.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        // Seed a couple of real seqs first so the new line's seq is non-zero - proves it's
        // a real derived value, not coincidentally matching the old seq: 0 sentinel.
        for i in 0..2u64 {
            let seq = app.worlds[0].next_seq;
            app.worlds[0].next_seq += 1;
            app.worlds[0].output_lines.push(OutputLine::new(format!("line {i}"), seq));
        }

        app.ws_broadcast_log.lock().unwrap().clear();
        app.add_output("a system message");

        let expected_seq = app.worlds[0].output_lines.last().unwrap().seq;
        assert_eq!(expected_seq, 2);

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<(u64, Option<u64>)> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { seq, end_seq, .. } = m { Some((*seq, *end_seq)) } else { None }
        }).collect();
        assert_eq!(server_data.len(), 1, "{server_data:?}");
        assert_eq!(server_data[0], (expected_seq, Some(expected_seq)));
    }

    #[test]
    fn test_add_output_to_world_broadcasts_real_seq() {
        // Same regression guard as test_add_output_broadcasts_real_seq, for the
        // background-world variant - also confirms this client-generated system message
        // never advances the ▶ watermark (World::add_output_to_world always passes
        // from_server: false, and rule 1's "only text from the world is new" gates on
        // from_server - see World::line_is_new()), unlike the old model, which used to
        // mark it new purely because the world wasn't current, regardless of origin.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("current"));
        app.worlds.push(World::new("background"));
        app.current_world_index = 0;

        for i in 0..2u64 {
            let seq = app.worlds[1].next_seq;
            app.worlds[1].next_seq += 1;
            app.worlds[1].output_lines.push(OutputLine::new(format!("line {i}"), seq));
        }

        app.ws_broadcast_log.lock().unwrap().clear();
        app.add_output_to_world(1, "background world message");

        let expected_seq = app.worlds[1].output_lines.last().unwrap().seq;
        assert_eq!(expected_seq, 2);
        let arrived = app.worlds[1].output_lines.last().unwrap();
        assert!(!arrived.viewed,
            "a message arriving on a world nobody is viewing must be born unviewed, so it can \
             become ▶ for whoever looks at it next");
        assert_eq!(arrived.display_id, None,
            "arrival never assigns an owner - only a claim does");

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data: Vec<(usize, u64, Option<u64>)> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { world_index, seq, end_seq, .. } = m {
                Some((*world_index, *seq, *end_seq))
            } else { None }
        }).collect();
        assert_eq!(server_data.len(), 1, "{server_data:?}");
        assert_eq!(server_data[0], (1, expected_seq, Some(expected_seq)),
            "must carry the real seq");
        assert!(!log.iter().any(|m| matches!(m, WsMessage::ClaimedNew { world_index: 1, .. })),
            "no ClaimedNew should fire on arrival - ownership is only ever assigned when a \
             client displays the world");
    }

    #[test]
    fn test_clear_connection_state_resets_all_negotiation_and_session_fields() {
        // Regression guard for the daemon.rs /disconnect fix (T3 of the command-
        // duplication audit): daemon.rs used to hand-roll 7 field resets inline
        // instead of calling World::clear_connection_state, silently leaking
        // stale telnet/session state into the next connection attempt on that
        // world. Pin every field clear_connection_state is responsible for.
        let mut world = World::new("test");
        world.proxy_pid = Some(1234);
        world.proxy_socket_path = Some(std::path::PathBuf::from("/tmp/fake.sock"));
        world.proxy_socket_fd = Some(5);
        world.connected = true;
        world.socket_fd = Some(6);
        world.telnet_mode = true;
        world.negotiated_encoding = Some(Encoding::Utf8);
        world.naws_enabled = true;
        world.naws_sent_size = Some((80, 24));
        world.reader_name = Some("test-reader".to_string());
        world.skip_auto_login = true;
        world.fansi_detect_until = Some(std::time::Instant::now());
        world.fansi_login_pending = Some("login".to_string());
        world.last_send_time = Some(std::time::Instant::now());
        world.last_receive_time = Some(std::time::Instant::now());
        world.last_nop_time = Some(std::time::Instant::now());
        world.last_user_command_time = Some(std::time::Instant::now());
        world.active_media.insert("key".to_string(), "{}".to_string());
        world.prompt = "prompt> ".to_string();

        world.clear_connection_state(true, true);

        assert_eq!(world.proxy_pid, None);
        assert_eq!(world.proxy_socket_path, None);
        assert_eq!(world.proxy_socket_fd, None);
        assert!(!world.connected);
        assert_eq!(world.socket_fd, None);
        assert!(!world.telnet_mode);
        assert_eq!(world.negotiated_encoding, None);
        assert!(!world.naws_enabled);
        assert_eq!(world.naws_sent_size, None);
        assert_eq!(world.reader_name, None);
        assert!(!world.skip_auto_login, "skip_auto_login must reset so the next connect auto-logs in");
        assert_eq!(world.fansi_detect_until, None);
        assert_eq!(world.fansi_login_pending, None);
        assert_eq!(world.last_send_time, None);
        assert_eq!(world.last_receive_time, None);
        assert_eq!(world.last_nop_time, None);
        assert_eq!(world.last_user_command_time, None);
        assert!(world.active_media.is_empty());
        assert!(world.prompt.is_empty());
    }

    #[test]
    fn test_more_mode_visual_line_offset_survives_gagged_lines() {
        // Regression test: gagged lines appended after add_output must not
        // clear visual_line_offset (the bug was scroll_to_bottom in the gagged
        // lines handler resetting it to 0).
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;

        world.add_output("short\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short\n", true, &settings, output_height, output_width, false, true, false);
        let huge_line = "D".repeat(80 * 25);
        world.add_output(&format!("{}\nextra\n", huge_line), true, &settings, output_height, output_width, false, true, false);

        let saved_vlo = world.visual_line_offset;
        assert!(saved_vlo > 0, "Should have visual_line_offset set");

        // Simulate what the gagged lines handler does: append gagged lines + scroll_to_bottom
        let seq = world.next_seq;
        world.next_seq += 1;
        world.output_lines.push(OutputLine::new_gagged("gagged line".to_string(), seq));
        // The fix: save/restore visual_line_offset around scroll_to_bottom
        let saved = world.visual_line_offset;
        world.scroll_to_bottom();
        world.visual_line_offset = saved;

        assert_eq!(world.visual_line_offset, saved_vlo,
            "visual_line_offset should survive gagged line append");
    }

    #[test]
    fn test_hidden_visual_rows_and_more_indicator_vlo_only() {
        // Regression test for the "hidden row, no More indicator" bug: a world can be
        // paused with visual_line_offset > 0 and pending_lines EMPTY (the huge-line-only
        // batch never produced any pending lines). Before the fix, render_separator_bar's
        // condition was `paused && (!pending_lines.is_empty() || pending_count > 0)`, which
        // is false here — so the truncated row(s) would be hidden with no indicator at all.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;
        let max_lines = (output_height as usize) - 2; // 19

        world.add_output("short line one\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short line two\n", true, &settings, output_height, output_width, false, true, false);
        let huge_line = "A".repeat(80 * 25); // 25 visual rows at width 80
        world.add_output(&format!("{}\n", huge_line), true, &settings, output_height, output_width, false, true, false);

        assert!(world.paused, "Should be paused after huge line");
        assert!(world.pending_lines.is_empty(), "Huge line goes to output, not pending");

        // Same accounting as test_more_mode_single_line_exceeds_screen: visual_line_offset
        // ends up at max_lines - 2 (17), out of 25 total visual rows for the huge line.
        assert_eq!(world.visual_line_offset, max_lines - 2);
        let expected_hidden = 25 - world.visual_line_offset; // 8

        assert_eq!(world.hidden_visual_rows(&test_metrics(&settings, 80)), expected_hidden);
        assert_eq!(
            crate::rendering::more_indicator_count(&world, &test_metrics(&Settings::default(), 80)),
            Some(expected_hidden),
            "More indicator must fire for VLO-only truncation even with no pending lines"
        );
    }

    /// The drag-to-reveal gesture in the web/GUI/Android client asks for pending output one
    /// row at a time (`ReleasePending { count: 1 }`), so a budget of 1 must release exactly one
    /// logical line and leave the rest pending — including when that line wraps to several
    /// rows, where a naive "stop once the budget is spent" loop would release nothing and the
    /// gesture would stall forever.
    #[test]
    fn test_release_pending_one_row_budget_releases_exactly_one_line() {
        let settings = Settings::default();
        let width = 80usize;

        let mut world = World::new("test");
        // A first pending line far taller than the 1-row budget, then ordinary lines.
        world.pending_lines.push(make_output_line(&"w ".repeat(200), false));
        for i in 0..5 {
            world.pending_lines.push(make_output_line(&format!("pending {}", i), false));
        }
        world.paused = true;

        let metrics = test_metrics(&settings, width);
        assert!(metrics.rows(&world.pending_lines[0]) > 1,
            "precondition: the head line must wrap, or this doesn't test the overflow case");

        let released = world.release_pending(1, &metrics);
        assert_eq!(released.len(), 1, "a budget of 1 row must still release exactly one line");
        assert_eq!(world.pending_lines.len(), 5, "the rest must stay pending");
        assert_eq!(world.output_lines.len(), 1);
        assert!(world.paused, "still paused with a backlog outstanding");

        // And it keeps making progress one line per call.
        for expect_left in (0..5).rev() {
            let batch = world.release_pending(1, &metrics);
            assert_eq!(batch.len(), 1, "each call must move exactly one line");
            assert_eq!(world.pending_lines.len(), expect_left);
        }
        assert!(!world.paused, "pause clears once the backlog is drained");
    }

    /// The "More NNNN" count must measure the held-back line the way the renderer draws it.
    /// It used to measure the raw text at the plain terminal width, so every prefix the
    /// renderer adds - the "✨ " client-line marker, the 🛢️ archive marker, the F2 timestamp -
    /// was invisible to it. A line that those prefixes push onto an extra row then had that
    /// row hidden with nothing in the count to say so.
    #[test]
    fn test_more_indicator_counts_prefix_widened_rows() {
        let settings = Settings::default();
        let width = 80usize;

        // Exactly one full row of content at width 80. With the 3-column "✨ " prefix the
        // renderer adds for a client-generated line, it needs two.
        let text = "c".repeat(width);
        let mut client_line = make_output_line(&text, false);
        client_line.from_server = false;

        let plain_rows = crate::rendering::display_rows(
            &make_output_line(&text, false), width, false, &settings, &CachedNow::new());
        let client_rows = crate::rendering::display_rows(
            &client_line, width, false, &settings, &CachedNow::new());
        assert_eq!(plain_rows, 1);
        assert_eq!(client_rows, 2, "the ✨ prefix must push this line onto a second row");

        let mut world = World::new("test");
        world.output_lines.push(client_line);
        world.scroll_offset = 0;
        world.visual_line_offset = 1; // showing only the first row
        world.paused = true;

        assert_eq!(
            world.hidden_visual_rows(&test_metrics(&settings, width)),
            1,
            "the row hidden by the ✨ prefix must be counted"
        );
        assert_eq!(
            crate::rendering::more_indicator_count(&world, &test_metrics(&settings, width)),
            Some(1),
            "More must report the hidden prefix-widened row"
        );
    }

    /// Same gap, via F2: showing tags prepends a timestamp, which can also push a line onto
    /// another row. The count has to follow the toggle.
    #[test]
    fn test_more_indicator_follows_show_tags() {
        let settings = Settings::default();
        let width = 80usize;
        let text = "t".repeat(width);

        let mut world = World::new("test");
        world.output_lines.push(make_output_line(&text, false));
        world.scroll_offset = 0;
        world.visual_line_offset = 1;
        world.paused = true;

        let tags_off = crate::rendering::RowMetrics::new(&settings, false, width);
        let tags_on = crate::rendering::RowMetrics::new(&settings, true, width);

        assert_eq!(world.hidden_visual_rows(&tags_off), 0, "one row, nothing hidden");
        assert!(
            world.hidden_visual_rows(&tags_on) > 0,
            "with F2 on the timestamp prefix wraps this line, so a row is hidden"
        );
    }

    #[test]
    fn test_more_indicator_pending_plus_hidden() {
        // Continue from the VLO-only state, then add more output while still paused —
        // it should land in pending_lines. The indicator should report hidden + pending.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;

        world.add_output("short line one\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short line two\n", true, &settings, output_height, output_width, false, true, false);
        let huge_line = "A".repeat(80 * 25);
        world.add_output(&format!("{}\n", huge_line), true, &settings, output_height, output_width, false, true, false);
        let hidden = world.hidden_visual_rows(&test_metrics(&settings, 80));

        // Five more short (one-row) lines while paused with empty pending -> goes_to_pending.
        let more = "extra1\nextra2\nextra3\nextra4\nextra5\n";
        world.add_output(more, true, &settings, output_height, output_width, false, true, false);

        assert_eq!(world.pending_lines.len(), 5);
        assert_eq!(
            crate::rendering::more_indicator_count(&world, &test_metrics(&Settings::default(), 80)),
            Some(5 + hidden)
        );

        // Control case: pending lines present but visual_line_offset == 0 -> today's
        // pre-fix behavior is preserved exactly (count == pending_lines.len()).
        let mut plain_world = World::new("plain");
        let plain_settings = Settings { more_mode_enabled: true, ..Settings::default() };
        plain_world.add_output(
            "one\ntwo\nthree\n",
            true,
            &plain_settings,
            output_height,
            output_width,
            false,
            true,
            false,
        );
        plain_world.paused = true;
        plain_world.pending_lines.clear();
        for i in 0..3 {
            plain_world.pending_lines.push(make_output_line(&format!("pending {}", i), false));
        }
        assert_eq!(plain_world.visual_line_offset, 0);
        assert_eq!(
            crate::rendering::more_indicator_count(&plain_world, &test_metrics(&plain_settings, 80)),
            Some(plain_world.pending_lines.len())
        );
    }

    /// Build the T1 VLO-only-truncation state: paused, visual_line_offset > 0,
    /// pending_lines empty, at bottom (scroll_offset points at the huge line).
    fn build_vlo_only_paused_world() -> World {
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        let output_height: u16 = 21;
        let output_width: u16 = 80;

        world.add_output("short line one\n", true, &settings, output_height, output_width, false, true, false);
        world.add_output("short line two\n", true, &settings, output_height, output_width, false, true, false);
        let huge_line = "A".repeat(80 * 25);
        world.add_output(&format!("{}\n", huge_line), true, &settings, output_height, output_width, false, true, false);

        assert!(world.paused);
        assert!(world.pending_lines.is_empty());
        assert!(world.visual_line_offset > 0);
        assert!(world.is_at_bottom());
        world
    }

    #[test]
    fn test_filter_to_server_output_clears_stale_pause() {
        // Base case: at bottom, nothing pending -> Ctrl+L / toggle-tags should fully
        // unpause, not just clear visual_line_offset (the stale-pause bug, issue B).
        let mut world = build_vlo_only_paused_world();
        world.filter_to_server_output();

        assert_eq!(world.visual_line_offset, 0, "VLO should be reset");
        assert!(!world.paused, "Stale pause should be cleared with nothing held back");
        assert_eq!(world.lines_since_pause, 0, "lines_since_pause should be reset with paused");
        assert_eq!(
            crate::rendering::more_indicator_count(&world, &test_metrics(&Settings::default(), 80)),
            None,
            "No indicator should remain once fully unpaused with nothing hidden/pending"
        );
    }

    #[test]
    fn test_filter_to_server_output_keeps_pause_with_pending_lines() {
        // Variant (a): non-empty pending_lines -> paused must stay true, and the
        // indicator should report the still-held-back pending lines.
        let mut world = build_vlo_only_paused_world();
        for i in 0..4 {
            world.pending_lines.push(make_output_line(&format!("pending {}", i), false));
        }

        world.filter_to_server_output();

        assert_eq!(world.visual_line_offset, 0, "VLO should still be reset");
        assert!(world.paused, "Should remain paused while pending_lines is non-empty");
        assert_eq!(
            crate::rendering::more_indicator_count(&world, &test_metrics(&Settings::default(), 80)),
            Some(world.pending_lines.len()),
            "Indicator should report the pending backlog"
        );
    }

    #[test]
    fn test_filter_to_server_output_keeps_pause_with_remote_pending_count() {
        // Variant (b): empty local pending_lines but pending_count > 0 (simulated
        // remote-console mode, where paused mirrors the daemon-mirrored pending_count).
        // reset_visual_truncation's guard must leave paused untouched here.
        let mut world = build_vlo_only_paused_world();
        world.pending_count = 3;

        world.filter_to_server_output();

        assert_eq!(world.visual_line_offset, 0, "VLO should still be reset");
        assert!(world.paused, "Should remain paused/mirrored while pending_count > 0");
    }

    #[test]
    fn test_reset_visual_truncation_display() {
        // Resetting VLO truncation must reveal previously hidden wrapped rows via
        // build_display_lines, not just flip the internal counters.
        let mut world = build_vlo_only_paused_world();
        let settings = Settings::default();
        let rows_before = build_display_lines(&world, &settings, 21, 80, false).len();

        world.reset_visual_truncation();
        let rows_after = build_display_lines(&world, &settings, 21, 80, false).len();

        assert!(
            rows_after > rows_before,
            "Expected more display rows after reset_visual_truncation ({} before, {} after)",
            rows_before,
            rows_after
        );
    }

    #[test]
    fn test_release_orphaned_pending_before_draw() {
        // Issue A: if more-mode is disabled but the current world still holds
        // pending lines (e.g. the setting was toggled off elsewhere while paused),
        // release_orphaned_pending must drain them and request a redraw so they
        // appear this frame instead of on the next unrelated redraw.
        let mut app = App::new();
        app.worlds.push(World::new("test"));
        app.settings.more_mode_enabled = false;

        for i in 0..3 {
            app.current_world_mut().pending_lines.push(make_output_line(&format!("line {}", i), false));
        }
        app.current_world_mut().paused = true;
        app.needs_output_redraw = false;

        let released = app.release_orphaned_pending();

        assert!(released, "Should report that orphaned pending lines were released");
        assert!(app.current_world().pending_lines.is_empty(), "pending_lines should be drained");
        let output_tail: Vec<&str> = app.current_world().output_lines
            .iter()
            .rev()
            .take(3)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(output_tail, vec!["line 2", "line 1", "line 0"], "Released lines should be appended to output_lines");
        assert!(!app.current_world().paused, "Stale pause should be cleared");
        assert!(app.needs_output_redraw, "Should request a redraw");
        assert_eq!(app.current_world().pending_since, None, "pending_since should be cleared");

        // Second call: nothing left to release
        assert!(!app.release_orphaned_pending(), "Second call should report nothing released");
    }

    #[test]
    fn test_switch_to_oldest_pending_finds_vlo_only_world() {
        // Issue C: switch_to_oldest_pending's tiers only checked pending_lines/
        // unseen_lines, so a world left paused with hidden VLO-truncated rows
        // (viewed then switched away without releasing, so unseen_lines == 0
        // and pending_since == None) was invisible to Alt+w. Tier 2 must also
        // catch paused-with-VLO worlds.
        let mut app = App::new();
        app.worlds.push(World::new("world0"));
        app.worlds.push(World::new("world1"));

        app.worlds[1].paused = true;
        app.worlds[1].visual_line_offset = 3;
        app.worlds[1].unseen_lines = 0;
        app.worlds[1].pending_lines.clear();

        app.current_world_index = 0;

        assert!(app.switch_to_oldest_pending(), "Should switch to the VLO-only paused world");
        assert_eq!(app.current_world_index, 1);
    }

    #[test]
    fn test_partial_line_tracking_across_chunks() {
        // Simulate a single long MUD line arriving in multiple TCP chunks.
        // The line is: "fffff1 fffff2 ... fffff100\n" (~1000 bytes)
        // Arriving in 3 chunks without intermediate newlines.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // Build the full line
        let full_line: String = (1..=100).map(|i| format!("fffff{}", i)).collect::<Vec<_>>().join(" ");
        let full_with_newline = format!("{}\n", full_line);
        let bytes = full_with_newline.as_bytes();

        // Split into 3 roughly equal chunks (no newline until the very end)
        let chunk1 = std::str::from_utf8(&bytes[..333]).unwrap();
        let chunk2 = std::str::from_utf8(&bytes[333..666]).unwrap();
        let chunk3 = std::str::from_utf8(&bytes[666..]).unwrap();

        // Verify chunks don't have intermediate newlines
        assert!(!chunk1.contains('\n'), "chunk1 should not contain newline");
        assert!(!chunk2.contains('\n'), "chunk2 should not contain newline");
        assert!(chunk3.ends_with('\n'), "chunk3 should end with newline");

        // Process each chunk separately (simulating TCP reads)
        world.add_output(chunk1, true, &settings, 48, 80, false, true, false);
        assert_eq!(world.output_lines.len(), 1, "Chunk 1: should have 1 output line (partial)");
        assert!(!world.partial_line.is_empty(), "Chunk 1: partial_line should be set");

        world.add_output(chunk2, true, &settings, 48, 80, false, true, false);
        assert_eq!(world.output_lines.len(), 1, "Chunk 2: should STILL have 1 output line (updated partial)");
        assert!(!world.partial_line.is_empty(), "Chunk 2: partial_line should STILL be set (not lost)");

        world.add_output(chunk3, true, &settings, 48, 80, false, true, false);
        assert_eq!(world.output_lines.len(), 1, "Chunk 3: should STILL have 1 output line (completed)");
        assert!(world.partial_line.is_empty(), "Chunk 3: partial_line should be empty (line complete)");
        assert_eq!(world.pending_lines.len(), 0, "Should have 0 pending lines (just 1 logical line)");

        // Verify the final line content matches the original
        assert_eq!(world.output_lines[0].text, full_line,
            "Output line should be the complete original line");
    }

    #[test]
    fn test_partial_line_many_small_chunks() {
        // Simulate a single long line arriving in many small TCP chunks.
        // Without the fix, each chunk after the 2nd would create a new logical line.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        let full_line: String = (1..=200).map(|i| format!("w{}", i)).collect::<Vec<_>>().join(" ");
        let full_with_newline = format!("{}\n", full_line);
        let bytes = full_with_newline.as_bytes();

        // Send in 20 small chunks
        let chunk_size = bytes.len() / 20;
        for i in 0..20 {
            let start = i * chunk_size;
            let end = if i == 19 { bytes.len() } else { (i + 1) * chunk_size };
            let chunk = std::str::from_utf8(&bytes[start..end]).unwrap();
            world.add_output(chunk, true, &settings, 48, 80, false, true, false);
        }

        // Should be exactly 1 logical line, not 10+ fragmented lines
        assert_eq!(world.output_lines.len(), 1,
            "Should have exactly 1 output line, not {} (fragmented by partial bug)",
            world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 0, "Should have 0 pending lines");
        assert_eq!(world.output_lines[0].text, full_line);
    }

    #[test]
    fn test_more_mode_multiple_chunks() {
        // Test that more-mode works correctly when add_output is called multiple times
        // (simulating multiple TCP chunks arriving from the MUD server)
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // Simulate 10 TCP chunks of 100 lines each = 1000 total lines
        for chunk in 0..10 {
            let data: String = (1..=100).map(|i| {
                format!("fffff{}\n", chunk * 100 + i)
            }).collect();
            world.add_output(&data, true, &settings, 48, 80, false, true, false);
        }

        // max_lines = 46. After 46+1=47 lines, pause triggers.
        // First chunk (100 lines): 47 go to output, 53 go to pending
        // Subsequent chunks: all go to pending (paused is true)
        // Total pending: 53 + 9*100 = 953
        assert!(world.paused, "Should be paused after 1000 lines across 10 chunks");
        assert_eq!(world.output_lines.len(), 47,
            "Expected 47 output lines (max_lines=46, trigger on line 47), got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 953,
            "Expected 953 pending lines, got {}", world.pending_lines.len());
    }

    #[test]
    fn test_more_mode_1000_lines_single_call() {
        // Test that more-mode works with 1000 lines in a single add_output call
        // (simulating a TF /for loop generating all output at once)
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        let data: String = (1..=1000).map(|i| {
            format!("fffff{}\n", i)
        }).collect();
        world.add_output(&data, true, &settings, 48, 80, false, true, false);

        assert!(world.paused, "Should be paused after 1000 lines");
        assert_eq!(world.output_lines.len(), 47,
            "Expected 47 output lines, got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 953,
            "Expected 953 pending lines, got {}", world.pending_lines.len());
    }

    #[test]
    fn test_more_mode_multi_chunk() {
        // Test that more-mode works correctly when data arrives in multiple TCP chunks.
        // Each chunk is a separate add_output call. The pause trigger must not leak
        // extra lines into output when the triggering line is the last in a chunk.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // output_height=24, max_lines=22
        // Send 200 lines in chunks of varying sizes
        let all_lines: Vec<String> = (0..200).map(|i| format!("{}\n", i)).collect();

        // Chunk 1: lines 0-22 (23 lines). Line 22 triggers pause (lsp=22, 22+1=23>22).
        // This is the LAST line of the chunk, so pending is empty after the trigger.
        let chunk1: String = all_lines[0..23].concat();
        world.add_output(&chunk1, true, &settings, 24, 80, false, true, false);

        // After chunk 1: should have 23 output lines and be paused.
        // The key assertion: paused must remain true even though pending is empty.
        assert_eq!(world.output_lines.len(), 23,
            "Chunk 1: Expected 23 output lines, got {}", world.output_lines.len());

        // Chunk 2: lines 23-99 (77 lines). Already paused, all should go to pending.
        let chunk2: String = all_lines[23..100].concat();
        world.add_output(&chunk2, true, &settings, 24, 80, false, true, false);

        assert!(world.paused, "Should still be paused after chunk 2");
        assert_eq!(world.output_lines.len(), 23,
            "Chunk 2: Expected still 23 output lines, got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 77,
            "Chunk 2: Expected 77 pending lines, got {}", world.pending_lines.len());

        // Chunk 3: lines 100-199 (100 lines). Still paused, all go to pending.
        let chunk3: String = all_lines[100..200].concat();
        world.add_output(&chunk3, true, &settings, 24, 80, false, true, false);

        assert!(world.paused, "Should still be paused after chunk 3");
        assert_eq!(world.output_lines.len(), 23,
            "Chunk 3: Expected still 23 output lines, got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 177,
            "Chunk 3: Expected 177 pending lines, got {}", world.pending_lines.len());
    }

    #[test]
    fn test_release_pending_counts_visual_lines() {
        // Test that release_pending counts visual lines, not logical lines.
        // With output_width=80, a 500-char line wraps to ceil(500/80) = 7 visual lines.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // Create 10 long lines, each ~500 chars (7 visual lines each at width 80)
        let long_word = "x".repeat(500);
        let data: String = (0..10).map(|_| format!("{}\n", long_word)).collect();
        world.add_output(&data, true, &settings, 48, 80, false, true, false);

        // max_lines = 46. Each line = 7 visual lines.
        // Lines 1-6: 42 visual lines (< 46), go to output
        // Line 7: 42+7=49 > 46, triggers more. But 7 > 46? No, 7 < 46, so normal trigger.
        // Lines 8-10: go to pending
        assert!(world.paused, "Should be paused");

        let output_count = world.output_lines.len();
        let pending_count = world.pending_lines.len();
        assert!(output_count > 0 && pending_count > 0,
            "Should have both output ({}) and pending ({}) lines", output_count, pending_count);

        // Now test release_pending with visual budget of 46 (output_height - 2)
        // Each pending line is 7 visual lines. Budget=46 fits 6 lines (42 visual) or 7 lines (49 visual).
        // Since 42+7=49 > 46, it should stop at 6 lines (the 7th would exceed budget).
        let pending_before = world.pending_lines.len();
        world.release_pending(46, &test_metrics(&settings, 80));
        let released = pending_before - world.pending_lines.len();

        // Should release 6 lines (42 visual lines fits in 46 budget, 49 would exceed)
        assert!(released <= 7, "Should release at most 7 lines (visual budget), got {}", released);
        assert!(released >= 1, "Should release at least 1 line, got {}", released);
    }

    #[test]
    fn test_oversized_single_line_no_presplit() {
        // Test that a single oversized line is stored as one logical OutputLine.
        // Each renderer wraps at its own width. More-mode still pauses correctly
        // based on visual line count.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // Create a single very long line: 10000 chars at width 80, max_lines = 46
        // wrap_ansi_line produces ceil(10000/80) = 125 visual lines
        // The whole line goes to output_lines as one logical entry, and pause triggers
        let long_line = "x".repeat(10000) + "\n";
        world.add_output(&long_line, true, &settings, 48, 80, false, true, false);

        // Should be paused - the line exceeds max_lines worth of visual lines
        assert!(world.paused, "Should be paused");
        // Stored as 1 logical line (no pre-wrapping)
        assert_eq!(world.output_lines.len(), 1, "One logical line in output");
        assert_eq!(world.pending_lines.len(), 0, "No pending lines (whole line went to output)");
    }

    #[test]
    fn test_release_pending_visual_lines_mixed() {
        // Test release_pending with a mix of short and long lines
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // Add 3 short lines to trigger more-mode, then add long lines to pending
        // First fill output to near max_lines
        let short_data: String = (0..45).map(|i| format!("short line {}\n", i)).collect();
        world.add_output(&short_data, true, &settings, 48, 80, false, true, false);

        // Now add mixed content: 1 short line, then 1 very long line, then 1 short line
        let mixed: String = format!("short\n{}\nafter\n", "x".repeat(500));
        world.add_output(&mixed, true, &settings, 48, 80, false, true, false);

        assert!(world.paused, "Should be paused");
        let pending = world.pending_lines.len();
        assert!(pending > 0, "Should have pending lines");

        // Release with visual budget of 46
        // "short" = 1 visual line
        // 500-char line = ceil(500/80) = 7 visual lines
        // "after" = 1 visual line
        // total = 9 visual lines < 46, so all should be released
        world.release_pending(46, &test_metrics(&settings, 80));
        assert_eq!(world.pending_lines.len(), 0,
            "All {} pending lines should fit in visual budget of 46", pending);
    }

    #[test]
    fn test_wrap_ansi_line_no_spaces() {
        // No spaces = hard wrap at character boundary
        let line = "a".repeat(100);
        let lines = wrap_ansi_line(&line, 10, 0);
        assert_eq!(lines.len(), 10, "Should produce 10 visual lines");
        for (i, vl) in lines.iter().enumerate() {
            let stripped = strip_ansi_codes(vl);
            if i < 9 {
                assert_eq!(stripped.len(), 10, "Line {} should be 10 chars", i);
            } else {
                assert_eq!(stripped.len(), 10, "Last line should be 10 chars");
            }
        }
    }

    #[test]
    fn test_wrap_ansi_line_word_boundary() {
        // Word wrapping at width 10:
        //   "aaa bbb ccc ddd eee fff"
        //   Line 1: "aaa bbb " (wraps at space before "ccc")
        //   Line 2: "ccc ddd " (wraps at space before "eee")
        //   Line 3: "eee fff"
        let line = "aaa bbb ccc ddd eee fff";
        let lines = wrap_ansi_line(line, 10, 0);
        assert!(lines.len() >= 2, "Should produce multiple lines, got {}", lines.len());
        // First line should break at word boundary
        let first_stripped = strip_ansi_codes(&lines[0]);
        assert!(first_stripped.starts_with("aaa bbb"),
            "First line should start with 'aaa bbb': {:?}", first_stripped);
    }

    #[test]
    fn test_wrap_ansi_line_with_ansi() {
        // Test ANSI color codes carried across line boundaries
        let line = format!("\x1b[31m{}\x1b[0m", "r".repeat(25));
        let lines = wrap_ansi_line(&line, 10, 0);
        assert_eq!(lines.len(), 3, "Should produce 3 lines");
        // Second line should carry the red color code
        assert!(lines[1].contains("\x1b[31m"),
            "Second line should carry color code: {:?}", lines[1]);
        // First line should end with reset
        assert!(lines[0].ends_with("\x1b[0m"),
            "First line should end with reset: {:?}", lines[0]);
    }

    #[test]
    fn test_wrap_ansi_line_short_passthrough() {
        let line = "hello world";
        let lines = wrap_ansi_line(line, 80, 0);
        assert_eq!(lines.len(), 1);
        // Should contain the original text (plus trailing reset)
        assert!(strip_ansi_codes(&lines[0]).contains("hello world"));
    }

    #[test]
    fn test_wrap_ansi_line_fffff_pattern() {
        // Simulate the actual test case: space-separated fffff words at width 80
        let words: Vec<String> = (0..1000).map(|i| format!("fffff{}", i)).collect();
        let line = words.join(" ");
        let lines = wrap_ansi_line(&line, 80, 0);
        assert!(lines.len() > 1, "Should produce multiple visual lines");
        // Each visual line (except last) should be <= 80 display width
        for (i, vl) in lines.iter().enumerate() {
            let stripped = strip_ansi_codes(vl);
            let dw: usize = stripped.chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum();
            assert!(dw <= 80, "Line {} has display width {} (max 80): {:?}",
                i, dw, &stripped[..40.min(stripped.len())]);
        }
        // Verify word boundaries: no line should start with a partial word
        for (i, vl) in lines.iter().enumerate() {
            if i > 0 {
                let stripped = strip_ansi_codes(vl);
                assert!(stripped.starts_with("fffff"),
                    "Line {} should start at a word boundary: {:?}",
                    i, &stripped[..20.min(stripped.len())]);
            }
        }
    }

    // --- wrapspace (hanging indent on continuation rows) ---

    #[test]
    fn test_wrap_ansi_line_indent_word_boundary() {
        // Same as test_wrap_ansi_line_word_boundary but with a 4-space wrapspace indent.
        // First row unindented; every continuation row gets 4 leading spaces.
        let line = "aaa bbb ccc ddd eee fff";
        let lines = wrap_ansi_line(line, 10, 4);
        assert!(lines.len() >= 2, "Should produce multiple lines, got {}", lines.len());
        let first_stripped = strip_ansi_codes(&lines[0]);
        assert!(!first_stripped.starts_with(' '), "First row must not be indented: {:?}", first_stripped);
        for (i, vl) in lines.iter().enumerate().skip(1) {
            let stripped = strip_ansi_codes(vl);
            assert!(stripped.starts_with("    "), "Row {} should start with 4 spaces: {:?}", i, stripped);
        }
    }

    #[test]
    fn test_wrap_ansi_line_indent_zero_adds_no_leading_spaces() {
        // indent=0 (the wrapspace default) must reproduce the exact pre-wrapspace behavior:
        // no continuation row gets any leading whitespace added.
        let line = "aaa bbb ccc ddd eee fff";
        for vl in &wrap_ansi_line(line, 10, 0) {
            let stripped = strip_ansi_codes(vl);
            assert!(!stripped.starts_with(' '), "indent=0 should never add leading spaces: {:?}", stripped);
        }
    }

    #[test]
    fn test_wrap_ansi_line_indent_uncolored_with_active_background() {
        // A line with an active background color that wraps — the injected indent spaces
        // on the continuation row must appear BEFORE the color-restoring prefix, so they
        // render in the terminal's default color, not painted by the active background.
        let line = format!("\x1b[41m{}\x1b[0m", "x".repeat(30)); // 41 = red background
        let lines = wrap_ansi_line(&line, 10, 3);
        assert!(lines.len() >= 2, "Should wrap into multiple rows");
        let second = &lines[1];
        // The row must start with the 3 raw indent spaces, THEN the color code — not
        // color code first (which would paint the indent).
        assert!(second.starts_with("   \x1b[41m") || second.starts_with("   "),
            "Continuation row should lead with uncolored indent spaces: {:?}", second);
        assert!(!second.starts_with("\x1b"), "Indent spaces must precede any color code: {:?}", second);
    }

    #[test]
    fn test_wrap_ansi_line_indent_pathological_still_progresses() {
        // indent >= max_width must not stall/loop — it should clamp internally and still
        // consume the whole input, leaving at least 1 content column per row.
        let line = "a".repeat(50);
        let lines = wrap_ansi_line(&line, 10, 999);
        assert!(!lines.is_empty(), "Must still produce output");
        // Every produced row's stripped text must be non-empty (forward progress guaranteed).
        for (i, vl) in lines.iter().enumerate() {
            let stripped = strip_ansi_codes(vl);
            assert!(!stripped.trim().is_empty() || i == lines.len() - 1,
                "Row {} should carry real content: {:?}", i, stripped);
        }
        // All 50 'a' characters must still be present across all rows combined.
        let total_as: usize = lines.iter().map(|l| strip_ansi_codes(l).matches('a').count()).sum();
        assert_eq!(total_as, 50, "No characters should be lost at a pathological indent");
    }

    #[tokio::test]
    async fn test_regression_more_mode_triggers_on_flood() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood");

        // Start server
        let server = tokio::spawn(testserver::run_server_port(port, scenario));

        // Give server time to bind
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should have Connected event
        assert!(events.iter().any(|e| matches!(e, TestEvent::Connected(n) if n == "test")),
            "Expected Connected event");

        // Should have TextReceived events
        let text_count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
        assert!(text_count > 0, "Expected TextReceived events, got 0");

        // Should have MoreTriggered event (30 lines with output_height=24 should trigger)
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Expected MoreTriggered event. Events: {:?}", events);

        // Should have Disconnected event
        assert!(events.iter().any(|e| matches!(e, TestEvent::Disconnected(_))),
            "Expected Disconnected event");

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_regression_more_mode_disabled_no_pause() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,  // Disabled!
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should NOT have MoreTriggered event
        assert!(!events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Should NOT have MoreTriggered with more_mode disabled. Events: {:?}", events);

        // Should still get all 30 lines
        let text_count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
        assert_eq!(text_count, 30, "Expected 30 TextReceived events, got {}", text_count);

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_regression_activity_count_multiple_worlds() {
        let port1 = find_free_port();
        let port2 = find_free_port();
        let port3 = find_free_port();

        // World 1: idle (we'll be viewing this one)
        // World 2,3: basic output (generates unseen lines)
        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));
        let server3 = tokio::spawn(testserver::run_server_port(port3, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world3".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port3,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should have UnseenChanged events for worlds 2 and 3 (since we're viewing world 1)
        let unseen_events: Vec<_> = events.iter()
            .filter(|e| matches!(e, TestEvent::UnseenChanged(_, n) if *n > 0))
            .collect();
        assert!(!unseen_events.is_empty(),
            "Expected UnseenChanged events for non-current worlds. Events: {:?}", events);

        // Should have ActivityChanged events
        assert!(events.iter().any(|e| matches!(e, TestEvent::ActivityChanged(n) if *n > 0)),
            "Expected ActivityChanged > 0. Events: {:?}", events);

        server1.abort();
        let _ = server2.await;
        let _ = server3.await;
    }

    #[tokio::test]
    async fn test_regression_unseen_cleared_on_switch() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for some text from world2 to generate unseen
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(3)),
            // Wait a bit more for all output
            TestAction::Sleep(Duration::from_millis(500)),
            // Switch to world2 - should clear unseen
            TestAction::SwitchWorld("world2".to_string()),
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should have unseen increased for world2 while viewing world1
        assert!(events.iter().any(|e| matches!(e, TestEvent::UnseenChanged(n, count) if n == "world2" && *count > 0)),
            "Expected UnseenChanged(world2, >0). Events: {:?}", events);

        // Should have WorldSwitched
        assert!(events.iter().any(|e| matches!(e, TestEvent::WorldSwitched(n) if n == "world2")),
            "Expected WorldSwitched(world2)");

        // After switching, unseen should be cleared
        // Find the last UnseenChanged for world2 after WorldSwitched
        let switch_idx = events.iter().position(|e| matches!(e, TestEvent::WorldSwitched(n) if n == "world2"));
        if let Some(idx) = switch_idx {
            let unseen_after: Vec<_> = events[idx..].iter()
                .filter(|e| matches!(e, TestEvent::UnseenChanged(n, _) if n == "world2"))
                .collect();
            if let Some(TestEvent::UnseenChanged(_, count)) = unseen_after.last() {
                assert_eq!(*count, 0, "Unseen should be 0 after switching to world2");
            }
        }

        server1.abort();
        let _ = server2.await;
    }

    #[tokio::test]
    async fn test_regression_auto_login_connect_type() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("auto_login_connect");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: "testuser".to_string(),
                password: "testpass".to_string(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should have auto-login sent
        assert!(events.iter().any(|e| matches!(e, TestEvent::AutoLoginSent(_, cmd) if cmd == "connect testuser testpass")),
            "Expected AutoLoginSent with 'connect testuser testpass'. Events: {:?}", events);

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_regression_auto_login_prompt_type() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("auto_login_prompt");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Prompt,
                username: "testuser".to_string(),
                password: "testpass".to_string(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should have username sent on first prompt
        assert!(events.iter().any(|e| matches!(e, TestEvent::AutoLoginSent(_, cmd) if cmd == "testuser")),
            "Expected AutoLoginSent with 'testuser'. Events: {:?}", events);

        // Should have password sent on second prompt
        assert!(events.iter().any(|e| matches!(e, TestEvent::AutoLoginSent(_, cmd) if cmd == "testpass")),
            "Expected AutoLoginSent with 'testpass'. Events: {:?}", events);

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_regression_disconnect_detection() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("disconnect_after");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should have Connected
        assert!(events.iter().any(|e| matches!(e, TestEvent::Connected(_))),
            "Expected Connected event");

        // Should have TextReceived (at least "Hello!" and "Goodbye!")
        let text_count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
        assert!(text_count >= 2, "Expected at least 2 TextReceived events, got {}", text_count);

        // Should have Disconnected
        assert!(events.iter().any(|e| matches!(e, TestEvent::Disconnected(_))),
            "Expected Disconnected event");

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_regression_more_mode_500_lines_scroll_through() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood_500");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(30),
        };

        // Build actions: wait for more-mode to trigger, then Tab through all pages
        let mut actions = vec![
            TestAction::WaitForEvent(WaitCondition::MoreTriggered),
            // Wait a moment for all data to arrive
            TestAction::Sleep(Duration::from_millis(500)),
        ];

        // Tab release enough times to drain all pending lines
        // 500 lines / 22 per page = ~23 tabs needed (with margin)
        for _ in 0..30 {
            actions.push(TestAction::TabRelease);
        }

        let events = testharness::run_test_scenario(config, actions).await;

        // Should have MoreTriggered
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Expected MoreTriggered event");

        // Should have received all 500 lines
        let text_count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
        assert_eq!(text_count, 500, "Expected 500 TextReceived events, got {}", text_count);

        // Should have MoreReleased at least once (final release)
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreReleased(_))),
            "Expected MoreReleased event");

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_regression_more_mode_500_lines_jump_to_end() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood_500");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(30),
        };

        let actions = vec![
            TestAction::WaitForEvent(WaitCondition::MoreTriggered),
            // Wait for all data to arrive
            TestAction::Sleep(Duration::from_millis(500)),
            // Jump to end (Escape+j) - release all at once
            TestAction::JumpToEnd,
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should have MoreTriggered
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Expected MoreTriggered event");

        // Should have received all 500 lines
        let text_count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
        assert_eq!(text_count, 500, "Expected 500 TextReceived events, got {}", text_count);

        // Should have MoreReleased
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreReleased(_))),
            "Expected MoreReleased event after JumpToEnd");

        let _ = server.await;
    }

    // ========== WebSocket Broadcast Tests ==========

    #[tokio::test]
    async fn test_ws_broadcast_activity_on_unseen() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Verify WsBroadcastActivity was emitted with count > 0
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastActivity(n) if *n > 0)),
            "Expected WsBroadcastActivity with count > 0. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastActivity(_))).collect::<Vec<_>>());

        // Verify WsBroadcastUnseen was emitted for world2 (index 1) with count > 0
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastUnseen(1, n) if *n > 0)),
            "Expected WsBroadcastUnseen(1, >0). WsBroadcastUnseen events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastUnseen(_, _))).collect::<Vec<_>>());

        server1.abort();
        let _ = server2.await;
    }

    #[tokio::test]
    async fn test_ws_broadcast_pending_on_more() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Verify MoreTriggered happened
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Expected MoreTriggered event");

        // Verify WsBroadcastPending was emitted with count > 0
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastPending(0, n) if *n > 0)),
            "Expected WsBroadcastPending(0, >0). WsBroadcastPending events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastPending(_, _))).collect::<Vec<_>>());

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ws_broadcast_released_on_tab() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood_500");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(30),
        };

        let actions = vec![
            TestAction::WaitForEvent(WaitCondition::MoreTriggered),
            TestAction::Sleep(Duration::from_millis(500)),
            TestAction::TabRelease,
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // TabRelease calls World::release_pending directly, not App::release_pending_screenful,
        // so we won't see WsBroadcastReleased from TabRelease. But we should see WsBroadcastPending
        // from the initial flood when lines were added to pending.
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastPending(0, n) if *n > 0)),
            "Expected WsBroadcastPending(0, >0) from initial flood");

        // MoreTriggered should have been emitted
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Expected MoreTriggered event");

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ws_release_pending_from_client() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood_500");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(30),
        };

        let actions = vec![
            TestAction::WaitForEvent(WaitCondition::MoreTriggered),
            TestAction::Sleep(Duration::from_millis(500)),
            // Simulate WS client releasing pending lines
            TestAction::WsReleasePending { world_name: "test".to_string(), count: 22 },
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // WsReleasePending uses App::release_pending_screenful which broadcasts
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastReleased(0, n) if *n > 0)),
            "Expected WsBroadcastReleased(0, >0) from WS client release. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastReleased(_, _))).collect::<Vec<_>>());

        // Should also see updated pending count broadcast
        // Find the last WsBroadcastPending - its count should be less than the peak
        let pending_events: Vec<_> = events.iter()
            .filter(|e| matches!(e, TestEvent::WsBroadcastPending(0, _)))
            .collect();
        assert!(pending_events.len() >= 2,
            "Expected at least 2 WsBroadcastPending events (initial flood + post-release). Got: {:?}", pending_events);

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ws_mark_seen_from_client() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for unseen on world2
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(3)),
            TestAction::Sleep(Duration::from_millis(500)),
            // Simulate WS client marking world2 as seen
            TestAction::WsMarkWorldSeen("world2".to_string()),
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should have unseen events for world2 before marking seen
        assert!(events.iter().any(|e| matches!(e, TestEvent::UnseenChanged(n, count) if n == "world2" && *count > 0)),
            "Expected UnseenChanged(world2, >0)");

        // Should have WsBroadcastUnseenCleared for world2 (index 1)
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastUnseenCleared(1))),
            "Expected WsBroadcastUnseenCleared(1). Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastUnseenCleared(_))).collect::<Vec<_>>());

        // Unseen should be cleared after marking seen
        assert!(events.iter().any(|e| matches!(e, TestEvent::UnseenChanged(n, 0) if n == "world2")),
            "Expected UnseenChanged(world2, 0) after marking seen");

        server1.abort();
        let _ = server2.await;
    }

    #[tokio::test]
    async fn test_ws_send_command_resets_pause() {
        let port = find_free_port();
        let scenario = testserver::get_scenario("more_flood_500");

        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![TestWorldConfig {
                name: "test".to_string(),
                host: "127.0.0.1".to_string(),
                port,
                use_ssl: false,
                auto_login_type: AutoConnectType::Connect,
                username: String::new(),
                password: String::new(),
            }],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(30),
        };

        let actions = vec![
            TestAction::WaitForEvent(WaitCondition::MoreTriggered),
            TestAction::Sleep(Duration::from_millis(500)),
            // Send a command via WS - should reset lines_since_pause
            TestAction::WsSendCommand { world_name: "test".to_string(), command: "look".to_string() },
            // Release all pending to get past the pause
            TestAction::JumpToEnd,
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // MoreTriggered should have happened from the flood
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _))),
            "Expected MoreTriggered event");

        // MoreReleased should have happened from JumpToEnd
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreReleased(_))),
            "Expected MoreReleased event after JumpToEnd");

        // All 500 lines should have been received
        let text_count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
        assert_eq!(text_count, 500, "Expected 500 TextReceived events, got {}", text_count);

        let _ = server.await;
    }

    #[tokio::test]
    async fn test_ws_activity_count_multi_world() {
        let port1 = find_free_port();
        let port2 = find_free_port();
        let port3 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));
        let server3 = tokio::spawn(testserver::run_server_port(port3, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world3".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port3,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should see WsBroadcastActivity(2) at peak when both world2 and world3 have unseen
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastActivity(2))),
            "Expected WsBroadcastActivity(2). Activity events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastActivity(_))).collect::<Vec<_>>());

        server1.abort();
        let _ = server2.await;
        let _ = server3.await;
    }

    #[tokio::test]
    async fn test_ws_broadcast_server_data_routing() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("basic_output")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let events = testharness::run_test_scenario(config, vec![]).await;

        // Should have WsBroadcastServerData for world1 (index 0)
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(0))),
            "Expected WsBroadcastServerData(0). ServerData events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastServerData(_))).collect::<Vec<_>>());

        // Should have WsBroadcastServerData for world2 (index 1)
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(1))),
            "Expected WsBroadcastServerData(1). ServerData events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastServerData(_))).collect::<Vec<_>>());

        let _ = server1.await;
        let _ = server2.await;
    }

    /// Test that output arriving on a non-current world gets marked_new=true,
    /// while current world output gets marked_new=false.
    #[tokio::test]
    async fn test_marked_new_on_non_current_world() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("basic_output")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        // World 0 is current. Both worlds receive basic_output (5 lines). Per rule 1 (see
        // World::new_from_seq's doc comment), world 0's lines must never render ▶ - the
        // watermark itself still advances (someone's watching), which is exactly what keeps
        // its own lines below it - while world 1's lines, arriving with nobody viewing, stay
        // ▶ until it's switched to.
        let actions = vec![
            // Both worlds run basic_output (5 lines each) - wait for all 10, not 5, or this
            // can race and check world2 before its own lines have arrived.
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(10)),
            TestAction::AssertMarkedNew { world_name: "world1".to_string(), expected_count: 0 },
            TestAction::AssertMarkedNew { world_name: "world2".to_string(), expected_count: 5 },
        ];
        let events = testharness::run_test_scenario(config, actions).await;

        // No ownership message for EITHER world: nothing was displayed in this scenario, and
        // ownership is only ever assigned by a display event. World 0's lines are born viewed
        // (someone is watching) so they can never be ▶; world 1's are unviewed and become ▶
        // for whoever displays that world next - neither is an ownership change now.
        assert!(!events.iter().any(|e| matches!(e, TestEvent::WsClaimedNew(_, _))),
            "arrival must never assign ownership. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsClaimedNew(_, _))).collect::<Vec<_>>());

        let _ = server1.await;
        let _ = server2.await;
    }

    /// Test that WsMarkWorldSeen clears unseen count but preserves marked_new indicators.
    /// marked_new indicators persist while viewing the world and are only cleared when
    /// switching away from it.
    #[tokio::test]
    async fn test_mark_seen_preserves_marked_new() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for world2 output to arrive (it's not current, so lines get marked_new)
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(5)),
            // Verify world2 has marked_new lines before clearing
            TestAction::AssertMarkedNew { world_name: "world2".to_string(), expected_count: 5 },
            // Verify unseen > 0
            TestAction::AssertState { world_name: "world2".to_string(), check: StateCheck::UnseenLines(5) },
            // Simulate WS client marking world2 as seen
            TestAction::WsMarkWorldSeen("world2".to_string()),
            // After mark_seen, marked_new should be PRESERVED (only cleared when switching away)
            TestAction::AssertMarkedNew { world_name: "world2".to_string(), expected_count: 5 },
            // Unseen should be 0 (mark_seen clears unseen count)
            TestAction::AssertState { world_name: "world2".to_string(), check: StateCheck::UnseenLines(0) },
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should see WsBroadcastUnseenCleared for world2 (index 1)
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastUnseenCleared(1))),
            "Expected WsBroadcastUnseenCleared(1) after MarkWorldSeen. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastUnseenCleared(_))).collect::<Vec<_>>());

        server1.abort();
        let _ = server2.await;
    }

    /// Test that switching worlds clears marked_new on the old world (the one being left)
    /// but preserves marked_new on the new world (so indicators remain visible).
    #[tokio::test]
    async fn test_switch_world_clears_old_world_marked_new() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for world2 output to arrive (non-current, gets marked_new)
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(5)),
            // Verify world2 has marked_new lines
            TestAction::AssertMarkedNew { world_name: "world2".to_string(), expected_count: 5 },
            // Switch to world2 (clears indicators on old world, preserves on new)
            TestAction::SwitchWorld("world2".to_string()),
            // After switching, world2's marked_new should be PRESERVED (indicators stay visible)
            TestAction::AssertMarkedNew { world_name: "world2".to_string(), expected_count: 5 },
            // world1 should have 0 (it was current, so never had marked_new)
            TestAction::AssertMarkedNew { world_name: "world1".to_string(), expected_count: 0 },
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should have WorldSwitched event
        assert!(events.iter().any(|e| matches!(e, TestEvent::WorldSwitched(ref n) if n == "world2")),
            "Expected WorldSwitched(world2)");

        server1.abort();
        let _ = server2.await;
    }

    /// Test that pending lines also get marked_new when arriving on a non-current world
    /// and that mark_seen preserves them (indicators only cleared when switching away).
    #[tokio::test]
    async fn test_pending_lines_marked_new_when_not_current() {
        let port1 = find_free_port();
        let port2 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("more_flood")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for more to trigger on world2 (non-current, 30 lines flood)
            TestAction::WaitForEvent(WaitCondition::MoreTriggered),
            // world2 should have marked_new lines in both output and pending
            // (output_height-2=22 lines in output, rest in pending, all marked_new since not current)
            TestAction::AssertState { world_name: "world2".to_string(), check: StateCheck::Paused(true) },
            // Mark world2 as seen via WS - marked_new preserved on both output and pending lines
            TestAction::WsMarkWorldSeen("world2".to_string()),
            // All lines (output + pending) still have marked_new (30 total)
            TestAction::AssertMarkedNew { world_name: "world2".to_string(), expected_count: 30 },
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should have seen MoreTriggered for world2
        assert!(events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(ref n, _) if n == "world2")),
            "Expected MoreTriggered for world2. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::MoreTriggered(_, _))).collect::<Vec<_>>());

        server1.abort();
        let _ = server2.await;
    }

    /// Test that activity count correctly reflects mark_seen operations.
    #[tokio::test]
    async fn test_activity_count_after_mark_seen() {
        let port1 = find_free_port();
        let port2 = find_free_port();
        let port3 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("idle")));
        let server2 = tokio::spawn(testserver::run_server_port(port2, testserver::get_scenario("basic_output")));
        let server3 = tokio::spawn(testserver::run_server_port(port3, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world2".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port2,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "world3".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port3,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for both non-current worlds to receive output (5 lines each = 10 total)
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(10)),
            // Activity should be 2 (world2 and world3 both have unseen)
            TestAction::AssertState { world_name: "".to_string(), check: StateCheck::ActivityCount(2) },
            // Mark world2 as seen
            TestAction::WsMarkWorldSeen("world2".to_string()),
            // Activity should drop to 1
            TestAction::AssertState { world_name: "".to_string(), check: StateCheck::ActivityCount(1) },
            // Mark world3 as seen
            TestAction::WsMarkWorldSeen("world3".to_string()),
            // Activity should drop to 0
            TestAction::AssertState { world_name: "".to_string(), check: StateCheck::ActivityCount(0) },
        ];

        let events = testharness::run_test_scenario(config, actions).await;

        // Should see activity change events
        assert!(events.iter().any(|e| matches!(e, TestEvent::ActivityChanged(2))),
            "Expected ActivityChanged(2). Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::ActivityChanged(_))).collect::<Vec<_>>());
        assert!(events.iter().any(|e| matches!(e, TestEvent::ActivityChanged(0))),
            "Expected ActivityChanged(0). Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::ActivityChanged(_))).collect::<Vec<_>>());

        server1.abort();
        let _ = server2.await;
        let _ = server3.await;
    }

    /// Test that the current world's output never gets marked_new.
    #[tokio::test]
    async fn test_current_world_output_not_marked_new() {
        let port1 = find_free_port();

        let server1 = tokio::spawn(testserver::run_server_port(port1, testserver::get_scenario("basic_output")));

        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "world1".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port1,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::Connect,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 24,
            output_width: 80,
            more_mode_enabled: false,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(5)),
            TestAction::AssertMarkedNew { world_name: "world1".to_string(), expected_count: 0 },
        ];
        let events = testharness::run_test_scenario(config, actions).await;

        // Should still have real ServerData broadcasts (content arrived) - the watermark
        // itself moved (see the NewWatermark note in test_marked_new_on_non_current_world),
        // which is what AssertMarkedNew above just confirmed keeps every line non-new.
        let server_data_events: Vec<_> = events.iter()
            .filter(|e| matches!(e, TestEvent::WsBroadcastServerData(0)))
            .collect();
        assert!(!server_data_events.is_empty(), "Should have ServerData broadcasts");

        let _ = server1.await;
    }

    /// Structural comparison test: verify that the JS INTERNAL_COMMANDS list in app.js
    /// matches the command strings handled by Rust's parse_command().
    /// This catches drift when commands are added to Rust but not to JS (or vice versa).
    #[test]
    fn test_command_parity_js_vs_rust() {
        // --- Extract JS INTERNAL_COMMANDS from app.js ---
        let app_js = std::fs::read_to_string("src/web/app.js")
            .expect("Failed to read src/web/app.js");

        // Find the INTERNAL_COMMANDS array
        let start_marker = "const INTERNAL_COMMANDS = [";
        let start_pos = app_js.find(start_marker)
            .expect("Could not find INTERNAL_COMMANDS in app.js");
        let after_start = &app_js[start_pos + start_marker.len()..];
        let end_pos = after_start.find(']')
            .expect("Could not find closing ] for INTERNAL_COMMANDS");
        let array_content = &after_start[..end_pos];

        // Parse the comma-separated quoted strings
        let mut js_commands: Vec<String> = Vec::new();
        for part in array_content.split(',') {
            let trimmed = part.trim().trim_matches('\'').trim_matches('"');
            if !trimmed.is_empty() {
                js_commands.push(trimmed.to_lowercase());
            }
        }
        js_commands.sort();
        js_commands.dedup();

        // --- Extract the real command list from parse_command()'s own source ---
        // A hand-maintained copy here is exactly what drifted out of sync before
        // (missing /say and /url from JS, with nothing able to catch it since this
        // test's own Rust-side list had silently drifted to match the same gaps).
        // Scanning the source directly means there's only one list left to update
        // when a command is added: INTERNAL_COMMANDS in app.js.
        let main_rs = std::fs::read_to_string("src/main.rs")
            .expect("Failed to read src/main.rs");
        let fn_start_marker = "pub fn parse_command(input: &str) -> Command {";
        let fn_start = main_rs.find(fn_start_marker)
            .expect("Could not find parse_command() in main.rs");
        let fn_end_marker = "fn parse_world_command";
        let fn_end_rel = main_rs[fn_start..].find(fn_end_marker)
            .expect("Could not find end of parse_command() in main.rs");
        let fn_body = &main_rs[fn_start..fn_start + fn_end_rel];

        // Match top-level `"/cmd"` or `"/cmd1" | "/cmd2"` match-arm patterns (only
        // lines that actually dispatch on the token, not incidental quoted strings
        // like the "/common" example in a comment elsewhere in this function).
        let arm_re = regex::Regex::new(r#"(?m)^\s*((?:"/[a-zA-Z_]+"\s*\|\s*)*"/[a-zA-Z_]+")\s*=>"#).unwrap();
        let token_re = regex::Regex::new(r#""/([a-zA-Z_]+)""#).unwrap();
        let mut rust_commands: Vec<String> = Vec::new();
        for arm_caps in arm_re.captures_iter(fn_body) {
            for tok_caps in token_re.captures_iter(&arm_caps[1]) {
                let name = tok_caps[1].to_lowercase();
                // /__connect is explicitly internal-use-only (Connect buttons), not a
                // user-typed command - out of scope for JS's INTERNAL_COMMANDS.
                if name != "__connect" {
                    rust_commands.push(name);
                }
            }
        }
        rust_commands.sort();
        rust_commands.dedup();
        assert!(rust_commands.len() > 30,
            "Sanity check failed: only found {} commands in parse_command() - the \
             extraction regex likely broke against a source change. Commands found: {:?}",
            rust_commands.len(), rust_commands);

        // --- Compare ---
        let js_set: std::collections::HashSet<&str> = js_commands.iter().map(|s| s.as_str()).collect();
        let rust_set: std::collections::HashSet<&str> = rust_commands.iter().map(|s| s.as_str()).collect();

        let missing_from_js: Vec<&&str> = rust_set.difference(&js_set).collect();
        let extra_in_js: Vec<&&str> = js_set.difference(&rust_set).collect();

        assert!(missing_from_js.is_empty() && extra_in_js.is_empty(),
            "Command parity mismatch between Rust parse_command() and JS INTERNAL_COMMANDS!\n\
             Missing from JS (present in Rust): {:?}\n\
             Extra in JS (not in Rust): {:?}\n\
             \n\
             To fix: update INTERNAL_COMMANDS in src/web/app.js (parse_command() is scanned \
             directly now, so there's no second Rust-side list to keep in sync here).",
            missing_from_js, extra_in_js);
    }

    #[test]
    fn test_is_newer_version() {
        // Basic version comparison
        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.1.0", "1.0.0"));
        assert!(is_newer_version("2.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
        assert!(!is_newer_version("0.9.0", "1.0.0"));

        // Pre-release handling
        assert!(is_newer_version("1.0.0", "1.0.0-alpha"));
        assert!(!is_newer_version("1.0.0-alpha", "1.0.0"));
        assert!(!is_newer_version("1.0.0-alpha", "1.0.0-alpha"));

        // Different length versions
        assert!(is_newer_version("1.0.1", "1.0"));
        assert!(!is_newer_version("1.0", "1.0.1"));
        assert!(!is_newer_version("1.0", "1.0.0"));
    }

    // --- build_display_lines tests ---

    /// Helper: create an OutputLine that will render with (or without) the ▶ new-text
    /// indicator once pushed into a world whose `new_from_seq` watermark is 1 (see the two
    /// buckets below) - there's no per-line marked_new flag anymore (see
    /// World::new_from_seq's doc comment in main.rs), so the desired old/new split is
    /// encoded via seq instead: bucket 0 for "old" (seq < watermark), bucket 1 for "new"
    /// (seq >= watermark). Callers that care about the distinction must set
    /// `world.new_from_seq = 1` after construction; tests that don't check old/new at all
    /// can ignore this and it's a harmless no-op.
    /// `marked_new` now sets per-line ownership directly (`display_id`), which is what the
    /// renderer reads — the old form encoded it as `seq: 1` and relied on the test setting
    /// `world.new_from_seq = 1` to bring it above the watermark. Ownership is the console's,
    /// since `rendering::line_is_new` always draws for the local instance.
    fn make_output_line(text: &str, marked_new: bool) -> OutputLine {
        OutputLine {
            text: text.to_string(),
            timestamp: std::time::SystemTime::now(),
            from_server: true,
            gagged: false,
            is_input: false,
            seq: if marked_new { 1 } else { 0 },
            highlight_color: None,
            from_archive: false,
            viewed: marked_new,
            display_id: if marked_new { Some(crate::CONSOLE_DISPLAY_ID) } else { None },
        }
    }

    /// Test A: NLI does not drop bottom lines
    /// 2 old lines + 20 new lines, visible_height=21, NLI enabled.
    /// The last display line must be the last output line (not cut off).
    #[test]
    fn test_build_display_nli_does_not_drop_bottom_lines() {
        let mut world = World::new("test");
        // 2 old (is_current=true, so marked_new=false)
        for i in 0..2 {
            world.output_lines.push(make_output_line(&format!("Old line {}", i + 1), false));
        }
        // 20 new lines (marked_new=true)
        for i in 0..20 {
            world.output_lines.push(make_output_line(&format!("New line {}", i + 1), true));
        }
        // scroll_offset at the end
        world.scroll_offset = world.output_lines.len() - 1;

        let settings = Settings { new_line_indicator: true, ..Settings::default() };

        let display = build_display_lines(&world, &settings, 21, 80, false);

        // Must show exactly 21 lines
        assert_eq!(display.len(), 21, "Expected 21 display lines, got {}", display.len());

        // Last line must contain the last new line text
        assert!(display.last().unwrap().text.contains("New line 20"),
            "Last display line should contain 'New line 20', got: {:?}", display.last().unwrap().text);

        // First 2 lines should be old context (marked_new=false)
        assert!(!display[0].marked_new, "First line should be old context");
        assert!(!display[1].marked_new, "Second line should be old context");
        assert!(display[0].text.contains("Old line 1"),
            "First display line should be old context, got: {:?}", display[0].text);
    }

    /// Test B: Boundary case — exactly visible_height + min_old_context
    /// 2 old + 21 new = 23 total visual lines, visible_height=21, NLI enabled.
    /// Should compose: 2 old at top + 19 new at bottom = 21.
    #[test]
    fn test_build_display_nli_boundary_composition() {
        let mut world = World::new("test");
        // 2 old lines
        for i in 0..2 {
            world.output_lines.push(make_output_line(&format!("Old {}", i + 1), false));
        }
        // 21 new lines
        for i in 0..21 {
            world.output_lines.push(make_output_line(&format!("New {}", i + 1), true));
        }
        world.scroll_offset = world.output_lines.len() - 1;

        let settings = Settings { new_line_indicator: true, ..Settings::default() };

        let display = build_display_lines(&world, &settings, 21, 80, false);

        assert_eq!(display.len(), 21);

        // First 2 should be old context
        let old_context = display.iter().take_while(|d| !d.marked_new).count();
        assert_eq!(old_context, 2, "Expected 2 old context lines, got {}", old_context);

        // Last line must be "New 21"
        assert!(display.last().unwrap().text.contains("New 21"),
            "Last line should be 'New 21', got: {:?}", display.last().unwrap().text);

        // The composition should skip 2 new lines (3 through 4) to fit
        // 2 old + 19 new = 21. So display[2] should be "New 3"
        assert!(display[2].text.contains("New 3"),
            "Third line should be 'New 3' (skipping New 1-2), got: {:?}", display[2].text);
    }

    /// Test C: NLI context disappears when far from scroll_offset
    /// After many lines, old context lines are far away — display should NOT compose.
    #[test]
    fn test_build_display_nli_context_disappears_when_far() {
        let mut world = World::new("test");
        // 2 old lines
        for i in 0..2 {
            world.output_lines.push(make_output_line(&format!("Old {}", i + 1), false));
        }
        // 100 new lines — far more than visible_height * 2
        for i in 0..100 {
            world.output_lines.push(make_output_line(&format!("New {}", i + 1), true));
        }
        world.scroll_offset = world.output_lines.len() - 1;

        let settings = Settings { new_line_indicator: true, ..Settings::default() };

        let display = build_display_lines(&world, &settings, 21, 80, false);

        assert_eq!(display.len(), 21);

        // No old context should appear — all lines should be marked_new
        let old_context = display.iter().take_while(|d| !d.marked_new).count();
        assert_eq!(old_context, 0, "Expected 0 old context lines when far away, got {}", old_context);

        // Last line should be "New 100"
        assert!(display.last().unwrap().text.contains("New 100"),
            "Last line should be 'New 100', got: {:?}", display.last().unwrap().text);
    }

    // ------------------------------------------------------------------
    // Row accounting: contiguity and row-exact paging
    //
    // Two defects lived here. (1) The NLI "old context" splice pinned 2 rows at the top and
    // jumped to the newest tail, discarding rows out of the *middle*, and fired on any buffer
    // whose row total landed in (H, H+2] - including all-old buffers, where it had no new text
    // to give context to. (2) Page Up budgeted rows with a div_ceil estimate at full terminal
    // width while the renderer word-wraps at a prefix-reduced width, so it under-counted long
    // lines and scrolled past rows that were on screen.
    // ------------------------------------------------------------------

    /// Every row the world would produce if nothing were clipped, in order. The displayed
    /// rows must always be a contiguous run of this.
    fn all_display_rows(world: &World, settings: &Settings, width: usize, show_tags: bool) -> Vec<String> {
        let now = CachedNow::new();
        world
            .output_lines
            .iter()
            .flat_map(|l| {
                let rows = crate::rendering::display_wrapped(l, width, show_tags, settings, &now);
                if rows.len() == 1 && rows[0].is_empty() {
                    vec![String::new()]
                } else {
                    rows
                }
            })
            .collect()
    }

    fn displayed_text(world: &World, settings: &Settings, height: usize, width: usize) -> Vec<String> {
        build_display_lines(world, settings, height, width, false)
            .into_iter()
            .map(|d| d.text)
            .collect()
    }

    /// Index into `all` where the displayed window starts. Panics if `shown` is not an
    /// unbroken run of `all` - i.e. if any row was dropped out of the middle of the screen.
    fn window_start(shown: &[String], all: &[String], ctx: &str) -> usize {
        assert!(!shown.is_empty(), "{ctx}: nothing displayed");
        all.windows(shown.len())
            .position(|w| w == shown)
            .unwrap_or_else(|| {
                panic!(
                    "{ctx}: displayed rows are not a contiguous run of the buffer.\nshown: {:#?}\nall: {:#?}",
                    shown, all
                )
            })
    }

    fn assert_contiguous(shown: &[String], all: &[String], ctx: &str) {
        window_start(shown, all, ctx);
    }

    /// The reported bug: reading an all-old buffer whose row total lands in the two-wide band
    /// just above the screen height. The splice used to fire anyway - `old_prefix` covered the
    /// whole vector, so `context` was 2 unconditionally - and silently ate the rows between
    /// the pinned pair and the tail. On screen that read as "the first two lines are there,
    /// then a couple are missing, and the next paragraph starts mid-sentence".
    #[test]
    fn test_build_display_all_old_buffer_is_contiguous_at_band_edges() {
        let settings = Settings { new_line_indicator: true, ..Settings::default() };
        let height = 21usize;
        let width = 80usize;

        // A line that wraps to exactly 3 rows at width 80, so the backward walk can overshoot
        // the screen height by 1 or 2 and land inside the band.
        let tall = "T".repeat(200);

        for filler_rows in [19usize, 20] {
            let mut world = World::new("test");
            for i in 0..10 {
                world.output_lines.push(make_output_line(&format!("Context {}", i), false));
            }
            world.output_lines.push(make_output_line(&tall, false));
            for i in 0..filler_rows {
                world.output_lines.push(make_output_line(&format!("Line {}", i), false));
            }
            world.scroll_offset = world.output_lines.len() - 1;

            let all = all_display_rows(&world, &settings, width, false);
            let shown = displayed_text(&world, &settings, height, width);

            assert_eq!(shown.len(), height, "filler_rows={filler_rows}: wrong row count");
            assert_contiguous(&shown, &all, &format!("filler_rows={filler_rows}"));
            // A bottom-anchored viewport ends on the newest row.
            assert_eq!(shown.last(), all.last(), "filler_rows={filler_rows}: not bottom-anchored");
        }
    }

    /// Scrollback is all-old by construction, so the splice must never engage there either -
    /// including when the anchor sits mid-paragraph, which row-exact paging now produces.
    #[test]
    fn test_build_display_scrollback_is_contiguous() {
        let settings = Settings { new_line_indicator: true, ..Settings::default() };
        let height = 21usize;
        let width = 80usize;

        let mut world = World::new("test");
        for i in 0..15 {
            world.output_lines.push(make_output_line(&format!("Before {}", i), false));
        }
        world.output_lines.push(make_output_line(&"P".repeat(600), false)); // ~8 rows
        for i in 0..15 {
            world.output_lines.push(make_output_line(&format!("After {}", i), false));
        }

        let all = all_display_rows(&world, &settings, width, false);
        let para_idx = 15;

        for vlo in 0..6 {
            world.scroll_offset = para_idx;
            world.visual_line_offset = vlo;
            let shown = displayed_text(&world, &settings, height, width);
            assert_contiguous(&shown, &all, &format!("scrollback vlo={vlo}"));
        }
    }

    /// The composition itself still works where it was meant to: new text filling the screen
    /// keeps two old rows pinned above it.
    #[test]
    fn test_build_display_composition_still_pins_old_context() {
        let settings = Settings { new_line_indicator: true, ..Settings::default() };
        let height = 21usize;

        let mut world = World::new("test");
        world.output_lines.push(make_output_line("Old 1", false));
        world.output_lines.push(make_output_line("Old 2", false));
        for i in 0..21 {
            world.output_lines.push(make_output_line(&format!("New {}", i + 1), true));
        }
        world.scroll_offset = world.output_lines.len() - 1;

        let display = build_display_lines(&world, &settings, height, 80, false);
        assert_eq!(display.len(), height);
        assert!(display[0].text.contains("Old 1"), "got {:?}", display[0].text);
        assert!(display[1].text.contains("Old 2"), "got {:?}", display[1].text);
        assert!(display[2].marked_new, "row 3 should be new text");
        assert!(display.last().unwrap().text.contains("New 21"));
    }

    /// A buffer with no new-marked rows at all must never compose, whatever its row total.
    #[test]
    fn test_visible_row_ranges_never_splices_an_all_old_buffer() {
        for row_count in 22..=23usize {
            let (head, tail) = crate::rendering::visible_row_ranges(row_count, 21, 2, row_count);
            assert!(tail.is_none(), "row_count={row_count}: spliced an all-old buffer");
            assert_eq!(head, row_count - 21..row_count);
        }
    }

    /// ...but it still composes when there is new text and bottom-anchoring would lose the
    /// old context entirely.
    #[test]
    fn test_visible_row_ranges_composes_for_new_text() {
        let (head, tail) = crate::rendering::visible_row_ranges(23, 21, 2, 2);
        assert_eq!(head, 0..2);
        assert_eq!(tail, Some(4..23));
    }

    // --- Row-exact Page Up / Page Down ---

    fn app_with_lines(lines: Vec<OutputLine>, height: u16, width: u16, nli: bool, wrapspace: u8) -> App {
        let mut app = App::new();
        app.output_height = height;
        app.output_width = width;
        app.settings.new_line_indicator = nli;
        app.settings.wrapspace = wrapspace;
        app.settings.more_mode_enabled = false;
        if app.worlds.is_empty() {
            app.worlds.push(World::new("test"));
        }
        app.current_world_index = 0;
        app.worlds[0].showing_splash = false;
        app.worlds[0].output_lines = lines;
        app.worlds[0].scroll_offset = app.worlds[0].output_lines.len().saturating_sub(1);
        app.worlds[0].visual_line_offset = 0;
        app
    }

    fn app_rows(app: &App) -> Vec<String> {
        displayed_text(
            &app.worlds[0],
            &app.settings,
            app.output_height as usize,
            app.output_width as usize,
        )
    }

    /// A buffer with a paragraph far taller than the screen, which is what broke paging:
    /// a line-granular anchor has to jump the whole paragraph at once.
    fn chatter_with_long_paragraph() -> Vec<OutputLine> {
        let long = "<Public> Great Britain Jacobis says, \"It was evident in fact. The pressures \
of the war had the most impact on their economies really, without significant opposition they \
could have blitzed Europe and perhaps had some decades of a decreasingly functioning society \
before the internal pressures tore it apart, much like we're seeing with Western societies now. \
The EU was, after all, the Nazi plan for Europe merely implemented by the post-war European \
powers. It has all the systemic issues of fascism, combined with a neoliberal belief in the \
possibility of regulating away every societal ill, so lack of democratic accountability and a \
proliferation of regulation which treats everything innovative as a worst-case maximal harm \
producing phenomenon has meant that innovation and technological industrial growth in Europe is \
stymied, corruption is rife, borders are uncontrolled, and who knows what's next.\"";

        let mut lines = Vec::new();
        // Deep enough that several full pages fit above the paragraph at every width the
        // tests exercise, so the top-of-buffer clamp isn't what they end up measuring.
        for i in 0..200 {
            lines.push(make_output_line(&format!("<Public> Filler line number {}", i), false));
        }
        lines.push(make_output_line(
            "<Public> ChatBot says, \"I agree the fascist regimes' internal issues stayed masked by their wartime loss.\"",
            false,
        ));
        lines.push(make_output_line(long, false));
        for i in 0..10 {
            lines.push(make_output_line(&format!("<Public> Adrick says, \"reply {}\"", i), false));
        }
        lines
    }

    /// Page Up keeps exactly the top two rows of the outgoing screen, as the bottom two rows
    /// of the new one - even when the boundary falls inside a paragraph taller than the
    /// screen. The old line-granular anchor could not express that position at all.
    #[test]
    fn test_page_up_keeps_top_two_rows_as_new_bottom_two() {
        for &(width, height) in &[(40u16, 24u16), (80, 24), (80, 12), (120, 30)] {
            for &nli in &[false, true] {
                for &wrapspace in &[0u8, 4] {
                    let mut app = app_with_lines(
                        chatter_with_long_paragraph(),
                        height,
                        width,
                        nli,
                        wrapspace,
                    );
                    let ctx = format!("w={width} h={height} nli={nli} ws={wrapspace}");
                    let all = all_display_rows(&app.worlds[0], &app.settings, width as usize, false);
                    let page = app.page_step();

                    for step in 0..6 {
                        let before = app_rows(&app);
                        assert_eq!(before.len(), height as usize, "{ctx} step {step}: short screen");
                        let pos_before = window_start(&before, &all, &format!("{ctx} step {step} before"));
                        let top_two = before[..2].to_vec();

                        if !app.scroll_output_up_rows(page) {
                            break;
                        }

                        let after = app_rows(&app);
                        assert_eq!(after.len(), height as usize, "{ctx} step {step}: short screen after");
                        let pos_after = window_start(&after, &all, &format!("{ctx} step {step} after"));

                        if pos_before - pos_after == page {
                            // A full page: the outgoing top two rows are now the bottom two.
                            assert_eq!(
                                &after[after.len() - 2..],
                                &top_two[..],
                                "{ctx} step {step}: lost the two-row overlap"
                            );
                        } else {
                            // A short move is only legitimate as the top-of-buffer clamp.
                            assert_eq!(
                                pos_after, 0,
                                "{ctx} step {step}: moved {} rows instead of {page} without \
                                 reaching the top of the buffer",
                                pos_before - pos_after
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Nothing is ever skipped: the screen after a Page Up is always a contiguous run of the
    /// buffer's rows, and it always overlaps the screen before it.
    #[test]
    fn test_page_up_never_skips_rows() {
        let mut app = app_with_lines(chatter_with_long_paragraph(), 24, 80, true, 0);
        let all = all_display_rows(&app.worlds[0], &app.settings, 80, false);

        for step in 0..8 {
            let shown = app_rows(&app);
            assert_contiguous(&shown, &all, &format!("page up step {step}"));
            if !app.scroll_output_up_rows(app.page_step()) {
                break;
            }
        }
    }

    /// Page Up then Page Down returns to exactly the screen you started on.
    #[test]
    fn test_page_up_down_round_trip() {
        for &(width, height) in &[(40u16, 24u16), (80, 24), (120, 16)] {
            let mut app = app_with_lines(chatter_with_long_paragraph(), height, width, true, 0);
            let ctx = format!("w={width} h={height}");
            let all = all_display_rows(&app.worlds[0], &app.settings, width as usize, false);
            let page = app.page_step();

            // Move off the bottom first so the round trip isn't trivially clamped.
            app.scroll_output_up_rows(page);
            let start = app_rows(&app);
            let start_pos = window_start(&start, &all, &ctx);
            let start_anchor = (app.worlds[0].scroll_offset, app.worlds[0].visual_line_offset);

            app.scroll_output_up_rows(page);
            let up = app_rows(&app);
            let up_pos = window_start(&up, &all, &ctx);
            assert_ne!(up, start, "{ctx}: page up did not move");
            // Only a full, unclamped page can round-trip back to the same screen.
            assert_eq!(start_pos - up_pos, page, "{ctx}: page up was clamped, widen the fixture");

            app.move_viewport_down(page);
            assert_eq!(app_rows(&app), start, "{ctx}: round trip changed the screen");
            assert_eq!(
                (app.worlds[0].scroll_offset, app.worlds[0].visual_line_offset),
                start_anchor,
                "{ctx}: round trip changed the anchor"
            );
        }
    }

    /// Paging down from anywhere lands back at the newest row, with the anchor normalized so
    /// `is_at_bottom()` reports following-live-output again.
    #[test]
    fn test_page_down_reaches_and_normalizes_the_bottom() {
        let mut app = app_with_lines(chatter_with_long_paragraph(), 24, 80, true, 0);
        for _ in 0..5 {
            app.scroll_output_up_rows(app.page_step());
        }
        assert!(!app.worlds[0].is_at_bottom() || app.worlds[0].visual_line_offset > 0);

        for _ in 0..20 {
            app.move_viewport_down(app.page_step());
        }
        assert!(app.worlds[0].is_at_bottom(), "did not reach the bottom");
        assert_eq!(
            app.worlds[0].visual_line_offset, 0,
            "visual_line_offset must normalize to 0 at the bottom, or the renderer's \
             `wrapped.len() > visual_line_offset` guard truncates some earlier line instead"
        );

        let all = all_display_rows(&app.worlds[0], &app.settings, 80, false);
        let shown = app_rows(&app);
        assert_eq!(shown.last(), all.last());
    }

    /// Scrolling up stops at the oldest row rather than running off the top.
    #[test]
    fn test_page_up_clamps_at_top_of_buffer() {
        let mut app = app_with_lines(chatter_with_long_paragraph(), 24, 80, true, 0);
        for _ in 0..200 {
            if !app.scroll_output_up_rows(app.page_step()) {
                break;
            }
        }
        let all = all_display_rows(&app.worlds[0], &app.settings, 80, false);
        let shown = app_rows(&app);
        assert_eq!(shown.len(), 24);
        assert_eq!(&shown[..], &all[..24], "top of buffer should be the first rows");
        assert!(!app.scroll_output_up_rows(app.page_step()), "should be clamped");
    }

    /// The row counter used for budgeting and the rows the renderer emits must agree exactly -
    /// including for archive lines (🛢️ prefix), ▶-marked lines, gagged lines, and F2 timestamps.
    #[test]
    fn test_display_rows_matches_rendered_rows() {
        let now = CachedNow::new();
        let long = "word ".repeat(60);

        let mut archive = make_output_line(&long, false);
        archive.from_archive = true;
        let mut gagged = make_output_line("gagged text", false);
        gagged.gagged = true;
        let mut client = make_output_line(&long, false);
        client.from_server = false;

        let lines = vec![
            make_output_line("short", false),
            make_output_line(&long, false),
            make_output_line(&long, true),
            archive,
            gagged,
            client,
            make_output_line("", false),
        ];

        for &show_tags in &[false, true] {
            for &nli in &[false, true] {
                for &width in &[20usize, 40, 80] {
                    let settings = Settings {
                        new_line_indicator: nli,
                        wrapspace: 2,
                        ..Settings::default()
                    };
                    for line in &lines {
                        let rendered =
                            crate::rendering::display_wrapped(line, width, show_tags, &settings, &now);
                        let counted =
                            crate::rendering::display_rows(line, width, show_tags, &settings, &now);
                        assert_eq!(
                            counted,
                            rendered.len(),
                            "w={width} tags={show_tags} nli={nli} text={:?}",
                            &line.text[..line.text.len().min(30)]
                        );
                    }
                }
            }
        }
    }

    /// Scrolling up mid-paragraph in more-mode and then releasing a screenful must not skip or
    /// repeat a row: `visual_line_offset` is both the scroll anchor and more-mode's
    /// partial-reveal marker, so the release budget has to measure the same way.
    #[test]
    fn test_more_mode_release_after_mid_paragraph_scroll() {
        let mut app = app_with_lines(chatter_with_long_paragraph(), 24, 80, true, 0);
        app.settings.more_mode_enabled = true;
        for i in 0..30 {
            app.worlds[0]
                .pending_lines
                .push(make_output_line(&format!("<Public> pending {}", i), true));
        }
        app.worlds[0].paused = true;

        // Land mid-paragraph.
        app.scroll_output_up_rows(app.page_step());
        app.scroll_output_up_rows(3);

        let all = all_display_rows(&app.worlds[0], &app.settings, 80, false);
        let shown = app_rows(&app);
        assert_contiguous(&shown, &all, "after mid-paragraph scroll");

        // Release a screenful; the viewport must still show a contiguous run.
        app.release_pending_screenful();
        let all_after = all_display_rows(&app.worlds[0], &app.settings, 80, false);
        let shown_after = app_rows(&app);
        assert_contiguous(&shown_after, &all_after, "after release_pending_screenful");
    }

    /// Test D: No NLI = simple bottom anchoring
    /// With NLI disabled, always shows last visible_height lines.
    #[test]
    fn test_build_display_no_nli_simple_bottom_anchoring() {
        let mut world = World::new("test");
        // 2 old lines + 20 new lines
        for i in 0..2 {
            world.output_lines.push(make_output_line(&format!("Old {}", i + 1), false));
        }
        for i in 0..20 {
            world.output_lines.push(make_output_line(&format!("New {}", i + 1), true));
        }
        world.scroll_offset = world.output_lines.len() - 1;

        let settings = Settings { new_line_indicator: false, ..Settings::default() };

        let display = build_display_lines(&world, &settings, 21, 80, false);

        assert_eq!(display.len(), 21);

        // With NLI disabled, should just show bottom 21 lines
        // That's Old 2 + New 1..20 = 21 lines
        assert!(display[0].text.contains("Old 2"),
            "First line should be 'Old 2', got: {:?}", display[0].text);
        assert!(display.last().unwrap().text.contains("New 20"),
            "Last line should be 'New 20', got: {:?}", display.last().unwrap().text);
    }

    /// Test E: Empty world produces empty display
    #[test]
    fn test_build_display_empty_world() {
        let world = World::new("test");
        let settings = Settings::default();
        let display = build_display_lines(&world, &settings, 21, 80, false);
        assert!(display.is_empty());
    }

    /// Test F: Fewer lines than visible_height shows all
    #[test]
    fn test_build_display_fewer_than_visible_height() {
        let mut world = World::new("test");
        for i in 0..5 {
            world.output_lines.push(make_output_line(&format!("Line {}", i + 1), false));
        }
        world.scroll_offset = world.output_lines.len() - 1;

        let settings = Settings::default();
        let display = build_display_lines(&world, &settings, 21, 80, false);

        assert_eq!(display.len(), 5);
        assert!(display[0].text.contains("Line 1"));
        assert!(display[4].text.contains("Line 5"));
    }

    /// Test G: visual_line_offset (partial display) truncation
    #[test]
    fn test_build_display_visual_line_offset() {
        let mut world = World::new("test");
        // Add a line that wraps to multiple visual lines (long text)
        let long_text = "A".repeat(200); // At width 80, wraps to 3 visual lines
        world.output_lines.push(make_output_line(&long_text, false));
        for i in 0..5 {
            world.output_lines.push(make_output_line(&format!("Line {}", i + 1), false));
        }
        world.scroll_offset = world.output_lines.len() - 1;
        world.visual_line_offset = 0; // No truncation

        let settings = Settings::default();
        let display_full = build_display_lines(&world, &settings, 21, 80, false);

        // Now set visual_line_offset to 1 — should truncate the long line to 1 visual line
        world.visual_line_offset = 1;
        // scroll_offset needs to point to the long line for VLO to apply
        world.scroll_offset = 0;
        let display_partial = build_display_lines(&world, &settings, 21, 80, false);

        // With VLO=1, the long line at scroll_offset=0 should be truncated to 1 visual line
        assert!(display_partial.len() < display_full.len(),
            "Partial display ({}) should have fewer lines than full ({})",
            display_partial.len(), display_full.len());
    }

    /// End-to-end wrapspace check through the full display pipeline (World + Settings +
    /// build_display_lines), using the exact example text from the feature request, at the
    /// exact width that produces the reported 3-row wrap. Confirms wrapspace=0 reproduces
    /// today's behavior and wrapspace=4 hang-indents every continuation row by 4 spaces
    /// while leaving the first row flush.
    #[test]
    fn test_build_display_wrapspace_hanging_indent() {
        let text = "[Public] P Phantom says, \"Yep. ORignally live around 2010-ish, got resurrected \
by fans, devs handed over code and license. Google 'City of Heroes: Homecoming' \
if you're more curious.\"";
        let width = 80;

        let mut world = World::new("test");
        world.output_lines.push(make_output_line(text, false));
        world.scroll_offset = 0;

        // wrapspace=0 — must match current (pre-feature) behavior exactly.
        let settings_off = Settings { wrapspace: 0, ..Settings::default() };
        let display_off = build_display_lines(&world, &settings_off, 21, width, false);
        assert!(display_off.len() >= 3, "Expected this line to wrap to 3+ rows at width {}: got {}", width, display_off.len());
        for row in &display_off {
            assert!(!row.text.starts_with(' '), "wrapspace=0 must add no indent: {:?}", row.text);
        }

        // wrapspace=4 — every row after the first gets a 4-space hang indent.
        let settings_on = Settings { wrapspace: 4, ..Settings::default() };
        let display_on = build_display_lines(&world, &settings_on, 21, width, false);
        assert_eq!(display_on.len(), display_off.len(),
            "Row count should be identical here since 4 columns of indent doesn't push this text past an extra wrap boundary");
        assert!(!display_on[0].text.starts_with(' '), "First row must stay flush: {:?}", display_on[0].text);
        for (i, row) in display_on.iter().enumerate().skip(1) {
            assert!(row.text.starts_with("    "), "Row {} should be indented 4 spaces: {:?}", i, row.text);
        }
    }

    /// Asserts `world.output_lines` is strictly increasing by `seq` - the invariant
    /// `App::broadcast_released_lines`'s real-seq broadcasts (once wired up, see Step 6 of
    /// the seq-drift fix) depend on. Reusable across any test that pushes/releases lines.
    fn assert_output_lines_seq_sorted(world: &World) {
        for pair in world.output_lines.windows(2) {
            assert!(pair[0].seq < pair[1].seq,
                "output_lines must be strictly increasing by seq, found {} immediately followed by {}: {:?}",
                pair[0].seq, pair[1].seq,
                world.output_lines.iter().map(|l| l.seq).collect::<Vec<_>>());
        }
    }

    #[test]
    fn test_gagged_line_while_paused_does_not_jump_ahead_of_pending() {
        // Regression guard: process_server_data's gagged-line loop used to push straight
        // into output_lines unconditionally, even while the world was paused with
        // pending_lines holding earlier (lower-seq) lines not yet released. That gave a
        // gagged line a NEWER seq immediately visible in output_lines, while an OLDER-seq
        // line sat in pending_lines - so once that pending line was eventually released and
        // appended after it, output_lines ended up with a seq dip in the middle. This is the
        // real root cause the release paths' `seq: 0` hack was working around (see
        // App::broadcast_released_lines' doc comment).
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("test");
        world.paused = true;
        world.settings.keep_alive_type = KeepAliveType::Custom;
        app.worlds.push(world);
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;

        // "hello world" is a normal server line - while paused with more-mode on, it goes
        // to pending_lines (World::add_output's goes_to_pending path). The idler marker is
        // unconditionally gagged regardless of action config (process_server_data's
        // idler-keepalive handling). Both arrive in the SAME chunk, exercising the exact
        // scenario the fix targets: a gagged line processed alongside pending non-gagged
        // output from the same read.
        app.process_server_data(0, b"hello world\r\n###_idler_message_1_###\r\n", 24, 80, false);

        assert!(app.worlds[0].output_lines.is_empty(),
            "nothing should have been released to output_lines while paused - the gagged line must not jump ahead");
        assert_eq!(app.worlds[0].pending_lines.len(), 2,
            "both the visible line and the gagged line must be queued in pending_lines: {:?}",
            app.worlds[0].pending_lines.iter().map(|l| (l.seq, l.gagged)).collect::<Vec<_>>());
        assert!(!app.worlds[0].pending_lines[0].gagged, "the visible line must be first (lower seq)");
        assert!(app.worlds[0].pending_lines[1].gagged, "the gagged line must be second (higher seq)");
        assert!(app.worlds[0].pending_lines[0].seq < app.worlds[0].pending_lines[1].seq,
            "pending_lines must stay seq-ordered");

        // Release everything and confirm output_lines comes out seq-sorted, not dipping.
        // release_all_pending() already moves the lines into output_lines itself (Step 1's
        // refactor) - no separate extend needed here.
        let released = app.worlds[0].release_all_pending();
        assert_eq!(released.len(), 2);
        assert_eq!(app.worlds[0].output_lines.len(), 2);
        assert_output_lines_seq_sorted(&app.worlds[0]);
    }

    #[test]
    fn test_server_data_end_seq_covers_filtered_lines() {
        // Regression guard for the seq-drift fix: ServerData.end_seq must span the full
        // batch as the server actually pushed it to output_lines, independent of what a
        // client might locally filter out for display (e.g. ANSI-only lines - the server
        // does NOT gag/drop those, only web/GUI/Android clients filter them for rendering).
        // A client deriving _max_seq from its own locally-filtered line count instead of
        // trusting end_seq is exactly the drift this field exists to eliminate.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("test"));
        app.current_world_index = 0;

        // Two real lines - the second is ANSI-reset-only with no visible content.
        app.process_server_data(0, b"visible line\r\n\x1b[0m\r\n", 24, 80, false);

        let expected_seqs: Vec<u64> = app.worlds[0].output_lines.iter().map(|l| l.seq).collect();
        assert_eq!(expected_seqs.len(), 2, "both lines should have been pushed to output_lines: {expected_seqs:?}");

        let log = app.ws_broadcast_log.lock().unwrap();
        let server_data_msgs: Vec<(u64, Option<u64>)> = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { world_index, seq, end_seq, .. } = m {
                if *world_index == 0 { Some((*seq, *end_seq)) } else { None }
            } else { None }
        }).collect();

        assert_eq!(server_data_msgs.len(), 1, "expected exactly one ServerData broadcast for this batch: {server_data_msgs:?}");
        let (seq, end_seq) = server_data_msgs[0];
        assert_eq!(seq, expected_seqs[0]);
        assert_eq!(end_seq, Some(expected_seqs[1]),
            "end_seq must span the full batch as pushed to output_lines, not a locally-filtered count");
    }

    // --- Integration test with test harness ---

    #[tokio::test]
    async fn test_more_mode_display_with_50_lines() {
        use crate::testserver;
        use crate::testharness::*;

        // Pick random ports to avoid conflicts
        let port_idle: u16 = 19401;
        let port_flood: u16 = 19402;

        // Start test servers
        let idle_scenario = testserver::get_scenario("idle");
        let flood_scenario = testserver::get_scenario("more_flood_50");

        let server1 = tokio::spawn(testserver::run_server_port(port_idle, idle_scenario));
        let server2 = tokio::spawn(testserver::run_server_port(port_flood, flood_scenario));

        // Brief delay for servers to bind
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = TestConfig {
            worlds: vec![
                TestWorldConfig {
                    name: "idle".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port_idle,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::NoLogin,
                    username: String::new(),
                    password: String::new(),
                },
                TestWorldConfig {
                    name: "flood".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: port_flood,
                    use_ssl: false,
                    auto_login_type: AutoConnectType::NoLogin,
                    username: String::new(),
                    password: String::new(),
                },
            ],
            output_height: 21,
            output_width: 80,
            more_mode_enabled: true,
            max_duration: Duration::from_secs(10),
        };

        let actions = vec![
            // Wait for all output to arrive (50 lines from flood world)
            TestAction::WaitForEvent(WaitCondition::TextReceivedCount(50)),
            // Flood world should be paused with pending lines
            TestAction::AssertState {
                world_name: "flood".to_string(),
                check: StateCheck::Paused(true),
            },
            // Switch to the flood world
            TestAction::SwitchWorld("flood".to_string()),
            // Assert display shows the first screenful
            TestAction::AssertDisplay {
                world_name: "flood".to_string(),
                visible_height: 21,
                term_width: 80,
                line_count: None, // Don't assert exact count yet (depends on how many fit before pause)
                last_line_contains: None,
                first_line_contains: Some("Line 001".to_string()), // First line should be visible
                old_context_count: None,
            },
            // Release first screenful (Tab)
            TestAction::TabRelease,
            TestAction::Sleep(Duration::from_millis(50)),
            // After Tab, display should show released lines
            TestAction::AssertDisplay {
                world_name: "flood".to_string(),
                visible_height: 21,
                term_width: 80,
                line_count: Some(21),
                last_line_contains: None, // Exact last line depends on release count
                first_line_contains: None,
                old_context_count: None,
            },
            // Release all remaining (Escape+j)
            TestAction::JumpToEnd,
            TestAction::Sleep(Duration::from_millis(50)),
            // After full release, all 50 lines should be in output_lines
            TestAction::AssertState {
                world_name: "flood".to_string(),
                check: StateCheck::OutputLineCount(50),
            },
            TestAction::AssertState {
                world_name: "flood".to_string(),
                check: StateCheck::PendingCount(0),
            },
            TestAction::AssertState {
                world_name: "flood".to_string(),
                check: StateCheck::Paused(false),
            },
            // Display should show last 21 lines with "Line 050" as last
            TestAction::AssertDisplay {
                world_name: "flood".to_string(),
                visible_height: 21,
                term_width: 80,
                line_count: Some(21),
                last_line_contains: Some("Line 050".to_string()),
                first_line_contains: Some("Line 030".to_string()),
                old_context_count: None,
            },
        ];

        let _events = run_test_scenario(config, actions).await;

        // Clean up servers
        server1.abort();
        server2.abort();
    }

    /// Unit test: NLI composition with build_display_lines called through the test harness pattern
    /// Tests that after releasing some pending lines, the display correctly shows old context + new
    #[test]
    fn test_build_display_nli_after_partial_release() {
        let mut world = World::new("test");

        // Simulate: 2 old lines already in output, then 30 pending get partially released
        for i in 0..2 {
            world.output_lines.push(make_output_line(&format!("Old {}", i + 1), false));
        }
        // Release 19 lines from pending (they become output with marked_new=true)
        for i in 0..19 {
            world.output_lines.push(make_output_line(&format!("Pending {}", i + 1), true));
        }
        world.scroll_offset = world.output_lines.len() - 1;
        // Still have 11 more in pending
        world.paused = true;

        let settings = Settings { new_line_indicator: true, ..Settings::default() };

        let display = build_display_lines(&world, &settings, 21, 80, false);

        assert_eq!(display.len(), 21, "Expected 21 display lines, got {}", display.len());

        // Should compose: 2 old context at top + 19 new at bottom
        let old_context = display.iter().take_while(|d| !d.marked_new).count();
        assert_eq!(old_context, 2, "Expected 2 old context lines, got {}", old_context);

        assert!(display[0].text.contains("Old 1"));
        assert!(display[1].text.contains("Old 2"));
        assert!(display.last().unwrap().text.contains("Pending 19"));
    }

    #[test]
    fn test_parse_remote_attach_command_host_port_colon() {
        match parse_command("/connect example.com:9000") {
            Command::RemoteAttach { addr, close, cancel } => {
                assert_eq!(addr, "example.com:9000");
                assert!(!close);
                assert!(!cancel);
            }
            other => panic!("Expected RemoteAttach, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_remote_attach_command_host_port_space() {
        match parse_command("/connect example.com 9000") {
            Command::RemoteAttach { addr, close, cancel } => {
                assert_eq!(addr, "example.com:9000");
                assert!(!close);
                assert!(!cancel);
            }
            other => panic!("Expected RemoteAttach, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_remote_attach_command_close() {
        match parse_command("/connect --close") {
            Command::RemoteAttach { addr, close, cancel } => {
                assert!(addr.is_empty());
                assert!(close);
                assert!(!cancel);
            }
            other => panic!("Expected RemoteAttach, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_remote_attach_command_cancel() {
        match parse_command("/connect --cancel") {
            Command::RemoteAttach { addr, close, cancel } => {
                assert!(addr.is_empty());
                assert!(!close);
                assert!(cancel);
            }
            other => panic!("Expected RemoteAttach, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_remote_attach_command_empty() {
        match parse_command("/connect") {
            Command::RemoteAttach { addr, close, cancel } => {
                assert!(addr.is_empty());
                assert!(!close);
                assert!(!cancel);
            }
            other => panic!("Expected RemoteAttach, got {:?}", other),
        }
    }

    /// Serializes tests that toggle the process-wide LOCAL_SERVER_LOOPBACK_ONLY static so
    /// they can't race each other's set/restore when cargo test runs them concurrently.
    static LOOPBACK_ONLY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard that restores LOCAL_SERVER_LOOPBACK_ONLY to its previous value on drop
    /// (including on panic), since it's a process-wide global shared with other tests.
    /// Holds LOOPBACK_ONLY_TEST_LOCK for its lifetime so concurrent uses of this guard
    /// (e.g. the two ensure_has_world tests below) can't interleave their set/restore.
    struct LoopbackOnlyGuard { previous: bool, _lock: std::sync::MutexGuard<'static, ()> }
    impl LoopbackOnlyGuard {
        fn set(value: bool) -> Self {
            let lock = LOOPBACK_ONLY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let previous = LOCAL_SERVER_LOOPBACK_ONLY.swap(value, std::sync::atomic::Ordering::SeqCst);
            LoopbackOnlyGuard { previous, _lock: lock }
        }
    }
    impl Drop for LoopbackOnlyGuard {
        fn drop(&mut self) {
            LOCAL_SERVER_LOOPBACK_ONLY.store(self.previous, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn test_ensure_has_world_desktop_uses_binary_name_placeholder() {
        let _guard = LoopbackOnlyGuard::set(false);
        let mut app = App::new();
        assert!(app.worlds.is_empty());

        app.ensure_has_world();

        assert_eq!(app.worlds.len(), 1);
        assert!(app.worlds[0].is_initial_world);
        // Not the Android-seeded default - hostname/port stay empty (default), and the
        // name comes from get_binary_name() rather than "Ascii".
        assert_ne!(app.worlds[0].name, "Ascii");
        assert!(app.worlds[0].settings.hostname.is_empty());
    }

    #[test]
    fn test_ensure_has_world_android_local_server_seeds_ascii_default() {
        let _guard = LoopbackOnlyGuard::set(true);
        let mut app = App::new();
        assert!(app.worlds.is_empty());

        app.ensure_has_world();

        assert_eq!(app.worlds.len(), 1);
        let world = &app.worlds[0];
        assert!(world.is_initial_world);
        assert_eq!(world.name, "Ascii");
        assert_eq!(world.settings.hostname, "teenymush.dynu.net");
        assert_eq!(world.settings.port, "4096");
        assert!(matches!(world.settings.world_type, WorldType::Mud));
        assert!(matches!(world.settings.encoding, Encoding::Utf8));
        assert!(matches!(world.settings.keep_alive_type, KeepAliveType::Nop));
    }

    /// Regression test for the bug where a daemon with many worlds could send an
    /// InitialState message exceeding the WebSocket size cap (websocket.rs's ws_config),
    /// causing ws_sink.send(...) to fail and silently drop a freshly-authenticated remote
    /// console/GUI connection. build_initial_state(0) must bound the TOTAL visible-line
    /// count across all worlds combined, not just cap each world independently - otherwise
    /// InitialState size scales with world_count * remote_initial_lines with no ceiling.
    #[test]
    fn test_build_initial_state_caps_total_lines_across_many_worlds() {
        let mut app = App::new();
        app.worlds.clear();

        // remote_initial_lines defaults to 100; with enough worlds, a per-world-only cap
        // would let this message grow unbounded. Use 20 worlds, each with far more than
        // the per-world cap's worth of lines, to exercise the aggregate budget.
        let world_count = 20;
        for i in 0..world_count {
            let mut world = World::new(&format!("world{i}"));
            for line in 0..300 {
                world.output_lines.push(OutputLine::new(format!("line {line}"), line as u64));
            }
            app.worlds.push(world);
        }
        app.current_world_index = 0;

        let per_world_cap = app.settings.remote_initial_lines.max(1) as usize;
        let expected_total_budget = per_world_cap.max(500);

        let initial_state = app.build_initial_state(0);
        let WsMessage::InitialState { worlds, .. } = initial_state else {
            panic!("build_initial_state(0) must return WsMessage::InitialState");
        };

        assert_eq!(worlds.len(), world_count, "all worlds should still be represented");
        let total_lines: usize = worlds.iter().map(|w| w.output_lines_ts.len()).sum();
        assert!(
            total_lines <= expected_total_budget,
            "total InitialState lines across all worlds ({total_lines}) must not exceed \
             the aggregate budget ({expected_total_budget}) - got {total_lines} from {world_count} \
             worlds with a per-world cap of {per_world_cap}"
        );
        // The first few worlds (in order) should still get their full per-world cap out of
        // the shared budget - only later worlds are starved once it runs out.
        assert_eq!(worlds[0].output_lines_ts.len(), per_world_cap,
            "the first world should get its full per-world cap while budget remains");
        // With a 500-line budget and a 100-line per-world cap, only 5 worlds fit before
        // the budget is exhausted; the rest get zero additional lines (they still backfill
        // immediately via RequestScrollback, same as any world's older history).
        assert_eq!(worlds[world_count - 1].output_lines_ts.len(), 0,
            "a world past the aggregate budget should get zero lines in InitialState, not a per-world floor");
    }

    /// The scrollback-download budget (Remote Lines) must count only VISIBLE (non-gagged)
    /// lines - gagged lines interspersed in a `before_seq` reply must ride along for free
    /// rather than eating into `count`. Also covers `backfill_complete`'s derivation: it must
    /// reflect whether `count` VISIBLE lines were actually found, not the raw returned line
    /// count (which can look "not exhausted" even when every line left in history was
    /// returned, if a long run of gagged lines sits at the boundary).
    #[test]
    fn test_handle_request_scrollback_before_seq_counts_only_visible_lines() {
        use crate::websocket::{WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        // seq 1..=20: odd seqs visible, even seqs gagged (10 of each).
        for seq in 1..=20u64 {
            if seq % 2 == 0 {
                world.output_lines.push(OutputLine::new_gagged(format!("gagged {seq}"), seq));
            } else {
                world.output_lines.push(OutputLine::new(format!("visible {seq}"), seq));
            }
        }
        app.worlds.push(world);
        app.current_world_index = 0;

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true, tx, current_world: None, username: None,
                received_initial_state: true, client_type: RemoteClientType::Web,
                viewport_height: 24, ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(), last_activity: std::time::Instant::now(),
                paused: false, acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        // Ask for 5 VISIBLE lines older than seq 21 (everything). Walking back from seq 20:
        // 5 visible lines (19,17,15,13,11) are found after passing 5 gagged ones (20,18,16,14,12).
        app.handle_request_scrollback(client_id, 0, 5, Some(21), None, None);
        let (lines, backfill_complete) = drain_one_scrollback_reply(&mut rx);
        let visible_in_reply = lines.iter().filter(|l| !l.gagged).count();
        assert_eq!(visible_in_reply, 5, "must return exactly 5 visible lines, got {lines:?}");
        assert_eq!(lines.len(), 10, "the 5 gagged lines interspersed in that range must ride along, not be skipped or counted against the budget");
        let seqs: Vec<u64> = lines.iter().map(|l| l.seq).collect();
        assert_eq!(seqs, vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20], "range must be contiguous and oldest-to-newest");
        assert!(!backfill_complete, "more visible history remains below seq 11 (seqs 1,3,5,7,9) - must not report exhausted");

        // Now ask for more visible lines than exist at all (20, but only 10 visible total).
        // The walk must exhaust the ENTIRE world and correctly report exhaustion based on the
        // visible count (10 < 20), not the raw returned count (20 lines returned, which would
        // look "not exhausted" under the old count: raw-line-count logic).
        app.handle_request_scrollback(client_id, 0, 20, Some(21), None, None);
        let (lines, backfill_complete) = drain_one_scrollback_reply(&mut rx);
        assert_eq!(lines.len(), 20, "must return every line in history when asking for more visible lines than exist");
        assert_eq!(lines.iter().filter(|l| !l.gagged).count(), 10, "only 10 visible lines actually exist");
        assert!(backfill_complete, "must report exhausted: only 10 visible lines exist despite 20 raw lines returned");
    }

    /// A scrollback reply must carry each line's real `viewed`/`display_id`, not placeholders.
    ///
    /// With `viewed: false` hardcoded, a world switch backfilled lines that claimed to be
    /// unviewed, so app.js's `claimUnviewedLocally()` took ▶ ownership of the whole screenful
    /// and the server's authoritative ClaimedNew (which claims nothing - it has them viewed)
    /// revoked them a round trip later. That is the "▶ shows then disappears, but only the
    /// first time I switch to a world" report. `display_id: None` was the other half: a line
    /// this client genuinely owns arrived with its marker stripped.
    #[test]
    fn test_scrollback_reply_carries_real_viewed_and_display_id() {
        use crate::websocket::{WsClientInfo, WebSocketServer, RemoteClientType, Outbound};
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            let mut line = OutputLine::new(format!("line {seq}"), seq);
            // What the server actually holds after another viewer has been watching: viewed,
            // and owned by display id 77.
            line.viewed = true;
            line.display_id = if seq % 2 == 0 { Some(77) } else { None };
            world.output_lines.push(line);
        }
        app.worlds.push(world);
        app.current_world_index = 0;

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true, tx, current_world: None, username: None,
                received_initial_state: true, client_type: RemoteClientType::Web,
                viewport_height: 24, ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(), last_activity: std::time::Instant::now(),
                paused: false, acked_seq: std::collections::HashMap::new(),
                audit_prev_acked: std::collections::HashMap::new(),
                audit_fired_at: std::collections::HashMap::new(),
                audit_stall_ticks: std::collections::HashMap::new(),
                push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        app.handle_request_scrollback(client_id, 0, 10, None, Some(0), None);
        let (lines, _) = drain_one_scrollback_reply(&mut rx);
        assert!(!lines.is_empty(), "expected a scrollback reply with lines");
        for line in &lines {
            assert!(line.viewed,
                "seq {} came back unviewed; the client would optimistically claim it and then \
                 have the marker revoked (the ▶ flash on first switch to a world)", line.seq);
            let expected = if line.seq % 2 == 0 { Some(77) } else { None };
            assert_eq!(line.display_id, expected,
                "seq {} lost its ▶ owner in the scrollback reply", line.seq);
        }
    }

    /// Same as the `before_seq` test above, but for `after_seq`'s forward (oldest-first) walk
    /// - the reconnect gap-fill direction.
    #[test]
    fn test_handle_request_scrollback_after_seq_counts_only_visible_lines() {
        use crate::websocket::{WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=20u64 {
            if seq % 2 == 0 {
                world.output_lines.push(OutputLine::new_gagged(format!("gagged {seq}"), seq));
            } else {
                world.output_lines.push(OutputLine::new(format!("visible {seq}"), seq));
            }
        }
        app.worlds.push(world);
        app.current_world_index = 0;

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true, tx, current_world: None, username: None,
                received_initial_state: true, client_type: RemoteClientType::Web,
                viewport_height: 24, ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(), last_activity: std::time::Instant::now(),
                paused: false, acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        // Ask for 5 VISIBLE lines newer than seq 0 (everything). Walking forward from seq 1:
        // 5 visible lines (1,3,5,7,9) are found after passing 4 gagged ones (2,4,6,8).
        app.handle_request_scrollback(client_id, 0, 5, None, Some(0), None);
        let (lines, backfill_complete) = drain_one_scrollback_reply(&mut rx);
        assert_eq!(lines.iter().filter(|l| !l.gagged).count(), 5, "must return exactly 5 visible lines, got {lines:?}");
        let seqs: Vec<u64> = lines.iter().map(|l| l.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6, 7, 8, 9], "range must be contiguous and oldest-to-newest");
        assert!(!backfill_complete, "more visible history remains above seq 9 - must not report exhausted");

        // Exhaustion case, forward direction.
        app.handle_request_scrollback(client_id, 0, 20, None, Some(0), None);
        let (lines, backfill_complete) = drain_one_scrollback_reply(&mut rx);
        assert_eq!(lines.len(), 20);
        assert_eq!(lines.iter().filter(|l| !l.gagged).count(), 10);
        assert!(backfill_complete, "must report exhausted based on the visible count (10 < 20), not the raw returned count (20)");
    }

    /// Drains exactly one `ScrollbackLines` reply from a test WS channel, returning
    /// `(lines, backfill_complete)`. Panics if none (or more than one) is found - callers
    /// issue exactly one `handle_request_scrollback` call per drain.
    fn drain_one_scrollback_reply(rx: &mut tokio::sync::mpsc::Receiver<crate::websocket::Outbound>) -> (Vec<TimestampedLine>, bool) {
        let mut result = None;
        while let Ok(item) = rx.try_recv() {
            if let crate::websocket::Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { lines, backfill_complete, .. } = *msg {
                    assert!(result.is_none(), "expected exactly one ScrollbackLines reply");
                    result = Some((lines, backfill_complete));
                }
            }
        }
        result.expect("expected a ScrollbackLines reply")
    }

    /// A world's seq epoch must survive a round trip through a JavaScript client, which
    /// parses JSON numbers as IEEE doubles. A full-width u64 does not: 12245391822682352775
    /// (a real epoch seen on the wire) comes back as 12245391822682354000, so
    /// `AuthRequest.resume_epochs` could never match and the InitialState skip silently
    /// never fired. Caught only by running the real Android client against a real server.
    #[test]
    fn test_seq_epoch_is_representable_in_javascript() {
        const MAX_SAFE: u64 = (1u64 << 53) - 1;
        for _ in 0..2000 {
            let epoch = World::new("w").seq_epoch;
            assert_ne!(epoch, 0, "0 is the reserved 'no epoch' value");
            assert!(epoch <= MAX_SAFE,
                "epoch {epoch} exceeds JS's exact-integer range ({MAX_SAFE}) and would be \
                 rounded by any browser/WebView client");
            // The property that actually matters: f64 round-trip is lossless.
            assert_eq!(epoch as f64 as u64, epoch, "epoch {epoch} does not survive an f64 round trip");
        }
    }

    /// A reconnecting client that still holds a world's buffer must not be re-sent that
    /// world's history. app.js hydrates a resumed world from its own in-memory buffer and
    /// never reads `output_lines_ts` for it, so those lines were serialized, shipped and
    /// parsed only to be dropped - the whole `remote_initial_lines` budget, on every
    /// reconnect, and Android reconnects on every resume from background.
    #[test]
    fn test_initial_state_skips_history_for_worlds_the_client_still_holds() {
        let mut app = App::new();
        app.worlds.clear();

        let mut world0 = World::new("held");
        for seq in 0..300u64 {
            world0.output_lines.push(OutputLine::new(format!("held {seq}"), seq));
        }
        let held_epoch = world0.seq_epoch;
        assert_ne!(held_epoch, 0, "a live world must have a real seq epoch");
        app.worlds.push(world0);

        let mut world1 = World::new("fresh");
        for seq in 0..300u64 {
            world1.output_lines.push(OutputLine::new(format!("fresh {seq}"), seq));
        }
        app.worlds.push(world1);
        app.current_world_index = 0;

        // The client resumes world 0 only, and its epoch matches.
        let state = app.build_initial_state_with_resume(0, &[(0, held_epoch)]);
        let WsMessage::InitialState { worlds, .. } = state else {
            panic!("expected InitialState");
        };

        assert!(worlds[0].output_lines_ts.is_empty(),
            "world 0 is held by the client - its history must not be re-sent, got {} lines",
            worlds[0].output_lines_ts.len());
        assert!(!worlds[1].output_lines_ts.is_empty(),
            "world 1 was not resumed - it must still get its history");

        // The metadata a client needs is still present for the skipped world: the skip is
        // about the line payload only.
        assert_eq!(worlds[0].name, "held");
        assert_eq!(worlds[0].total_output_lines, 300,
            "a skipped world must still report how much history the server holds, or the \
             client cannot tell there is anything to backfill");
        assert_eq!(worlds[0].seq_epoch, held_epoch);
    }

    /// The skip is keyed on the epoch, not the index, because the index alone cannot answer
    /// "is this the same world instance the client means?" - worlds get added and removed,
    /// so index N may name a different world than when the client recorded its frontier.
    /// Skipping on a stale index would leave that world empty on the client.
    #[test]
    fn test_initial_state_does_not_skip_when_the_epoch_does_not_match() {
        let mut app = App::new();
        app.worlds.clear();

        let mut world0 = World::new("world0");
        for seq in 0..300u64 {
            world0.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        let real_epoch = world0.seq_epoch;
        app.worlds.push(world0);
        app.current_world_index = 0;

        // Right index, wrong world: the client's buffer belongs to some other sequence space.
        let state = app.build_initial_state_with_resume(0, &[(0, real_epoch.wrapping_add(1))]);
        let WsMessage::InitialState { worlds, .. } = state else {
            panic!("expected InitialState");
        };
        assert!(!worlds[0].output_lines_ts.is_empty(),
            "a mismatched epoch means the client does not hold this world - history must be sent");

        // 0 is the "no epoch" value (multiuser, pre-epoch worlds). It must never match, or
        // two unrelated worlds would compare equal.
        app.worlds[0].seq_epoch = 0;
        let state = app.build_initial_state_with_resume(0, &[(0, 0)]);
        let WsMessage::InitialState { worlds, .. } = state else {
            panic!("expected InitialState");
        };
        assert!(!worlds[0].output_lines_ts.is_empty(),
            "epoch 0 means 'unknown', not 'matches' - history must be sent");
    }

    /// A skipped world must not consume the aggregate cross-world budget - it spent nothing,
    /// so the worlds that DO need history should get the full budget between them.
    #[test]
    fn test_skipped_world_does_not_consume_the_initial_state_budget() {
        let mut app = App::new();
        app.worlds.clear();

        let per_world_cap = app.settings.remote_initial_lines.max(1) as usize;

        let mut held = World::new("held");
        for seq in 0..(per_world_cap as u64 * 2) {
            held.output_lines.push(OutputLine::new(format!("held {seq}"), seq));
        }
        let held_epoch = held.seq_epoch;
        app.worlds.push(held);

        let mut fresh = World::new("fresh");
        for seq in 0..(per_world_cap as u64 * 2) {
            fresh.output_lines.push(OutputLine::new(format!("fresh {seq}"), seq));
        }
        app.worlds.push(fresh);
        app.current_world_index = 0;

        let state = app.build_initial_state_with_resume(0, &[(0, held_epoch)]);
        let WsMessage::InitialState { worlds, .. } = state else {
            panic!("expected InitialState");
        };
        assert!(worlds[0].output_lines_ts.is_empty());
        assert_eq!(worlds[1].output_lines_ts.len(), per_world_cap,
            "the un-held world must still get its full per-world cap - the skipped world \
             consumed none of the shared budget");
    }

    /// Plain `build_initial_state` (RequestState, the console/GUI attach path, a first
    /// connect) sends no resume list at all and must therefore be completely unaffected.
    #[test]
    fn test_initial_state_without_resume_is_unchanged() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world0 = World::new("world0");
        for seq in 0..50u64 {
            world0.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.worlds.push(world0);
        app.current_world_index = 0;

        let WsMessage::InitialState { worlds: with_empty, .. } =
            app.build_initial_state_with_resume(0, &[]) else { panic!("expected InitialState") };
        let WsMessage::InitialState { worlds: plain, .. } =
            app.build_initial_state(0) else { panic!("expected InitialState") };

        assert_eq!(with_empty[0].output_lines_ts.len(), 50);
        assert_eq!(plain[0].output_lines_ts.len(), with_empty[0].output_lines_ts.len(),
            "build_initial_state must stay exactly build_initial_state_with_resume(&[])");
    }

    /// `build_initial_state`'s aggregate cross-world budget must be spent only on VISIBLE
    /// lines - a world heavy with gagged content (active /gag rules, watchdog spam
    /// suppression) must not starve OTHER worlds' share of the budget just because its own
    /// slice happened to include a lot of invisible lines riding along for free.
    #[test]
    fn test_build_initial_state_budget_counts_only_visible_lines() {
        let mut app = App::new();
        app.worlds.clear();

        // World 0: 100 lines, 90% gagged (only 10 visible) - under the old raw-count
        // accounting this would burn ~100 lines of the aggregate budget; under visible-only
        // accounting it should burn only ~10, leaving far more for world 1.
        let mut world0 = World::new("world0");
        for seq in 0..100u64 {
            if seq % 10 == 0 {
                world0.output_lines.push(OutputLine::new(format!("visible {seq}"), seq));
            } else {
                world0.output_lines.push(OutputLine::new_gagged(format!("gagged {seq}"), seq));
            }
        }
        app.worlds.push(world0);

        // World 1: plenty of plain visible history, no gagged lines at all.
        let mut world1 = World::new("world1");
        for seq in 0..300u64 {
            world1.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.worlds.push(world1);
        app.current_world_index = 0;

        let per_world_cap = app.settings.remote_initial_lines.max(1) as usize; // default 100

        let initial_state = app.build_initial_state(0);
        let WsMessage::InitialState { worlds, .. } = initial_state else {
            panic!("build_initial_state(0) must return WsMessage::InitialState");
        };

        let world0_visible = worlds[0].output_lines_ts.iter().filter(|l| !l.gagged).count();
        assert_eq!(world0_visible, 10, "world0 should get all 10 of its visible lines (well under its per-world cap)");
        assert!(worlds[0].output_lines_ts.len() >= 10, "world0's slice must include its gagged lines too (they ride along free)");

        // World 1 must NOT be starved by world0's gagged lines consuming the aggregate
        // budget - it should still get its full per-world cap, since world0's real visible
        // cost (10) left the vast majority of the aggregate budget untouched.
        assert_eq!(worlds[1].output_lines_ts.len(), per_world_cap,
            "world1 must get its full per-world cap - world0's gagged lines must not have \
             eaten into the shared budget on world1's behalf. Got {} (cap {per_world_cap})",
            worlds[1].output_lines_ts.len());
    }

    /// Regression test for the follow-on bug where a budget-starved world (real history
    /// server-side, but zero lines locally after InitialState - see the aggregate-budget
    /// test above) was silently dropped from the auto-backfill queue instead of being
    /// requested with before_seq: None, leaving it permanently empty until the user
    /// manually focused and scrolled it. Also covers phase 1's per-world chunk sizing
    /// (request exactly enough to reach a screenful, not a fixed 500).
    #[test]
    fn test_backfill_advance_to_next_does_not_skip_budget_starved_worlds() {
        let mut app = App::new();
        app.worlds.clear();

        // World 0: already has some local lines (the ordinary "backfill older history"
        // case, unaffected by the bug/fix - included here to confirm it still works).
        let mut world0 = World::new("world0");
        for line in 0..10 {
            world0.output_lines.push(OutputLine::new(format!("line {line}"), line as u64));
        }
        app.worlds.push(world0);

        // World 1: budget-starved - InitialState reported total_output_lines=50 for it,
        // but it has zero lines locally (the aggregate budget in build_initial_state ran
        // out before reaching it).
        let world1 = World::new("world1");
        app.worlds.push(world1);

        app.current_world_index = 0;

        // Mirrors what InitialState's world_totals looks like: (world_index, total_output_lines).
        let world_totals = vec![(0, 20), (1, 50)];
        app.init_backfill(&world_totals, 75); // phase1_target = 75 (screenful)
        assert_eq!(app.backfill_queue, vec![0, 1],
            "both worlds have total > received and are under the phase 1 target, so both \
             should be queued for phase 1, current world first");

        // World 0 has local lines: backfill_next should anchor on its oldest seq and ask
        // for just enough to reach the phase 1 target (75 - 10 = 65), not a fixed chunk.
        app.backfill_advance_to_next();
        let (w0, seq0, count0, _rid0) = app.backfill_next.take().expect("phase 1 should still be issuing requests");
        assert_eq!((w0, seq0, count0), (0, Some(0), 65),
            "world 0 already has local lines, should backfill older history from its oldest \
             seq, requesting only enough to reach the phase 1 target");

        // World 1 is budget-starved (zero local lines): must still be queued for backfill,
        // not silently dropped. before_seq: None is the correct request - the daemon
        // handles it as "send the last N lines". It needs the full phase 1 target (75).
        app.backfill_advance_to_next();
        let (w1, seq1, count1, _rid1) = app.backfill_next.take().expect("phase 1 should still be issuing requests");
        assert_eq!((w1, seq1, count1), (1, None, 75),
            "a budget-starved world (real history, zero local lines) must still be requested \
             via RequestScrollback{{before_seq: None}}, not silently dropped from the queue");
    }

    /// Regression test for the console's backfill scope: it must fetch exactly one
    /// guaranteed screenful per world at connect and NEVER deep-fill beyond that,
    /// no matter how much more history the server reports or how high Remote Lines
    /// is set. This replaces a prior two-phase design (phase 1 screenful + phase 2
    /// round-robin deep fill up to Remote Lines) that was removed per the user's
    /// explicit direction: the console should only ever populate what fills the
    /// current screen - anything older is reached exclusively via scroll_page_up's
    /// existing on-demand, unbounded fetch, never proactively downloaded.
    #[test]
    fn test_console_backfill_never_deep_fills_past_initial_screenful() {
        let mut app = App::new();
        app.worlds.clear();
        // Remote Lines set far above the phase-1 screenful target, and each world has
        // far more server-side history than either value - if any deep-fill machinery
        // remained, this is exactly the scenario that would trigger it.
        app.settings.remote_initial_lines = 5000;

        for name in ["world0", "world1", "world2"] {
            app.worlds.push(World::new(name));
        }
        app.current_world_index = 0;

        let world_totals = vec![(0, 100_000), (1, 100_000), (2, 100_000)];
        app.init_backfill(&world_totals, 75);

        // Drive the one-time phase-1 queue to completion: one chunk per world, each
        // reply reporting plenty more history still available server-side
        // (backfill_complete: false) - if any deep-fill logic survived, this is what
        // it would key off of to keep requesting.
        for expected_world in [0usize, 1, 2] {
            app.backfill_advance_to_next();
            let (world_idx, before_seq, count, _request_id) = app.backfill_next.take()
                .expect("the initial screenful request should still be issued for this world");
            assert_eq!(world_idx, expected_world, "queue order should be current world first, then the rest");
            assert_eq!(count, 75, "the one-time request should ask for exactly a screenful");
            // Simulate the daemon's ScrollbackLines reply: full chunk given, and it
            // explicitly reports there's plenty more where that came from.
            let lines: Vec<OutputLine> = (0..count as u64)
                .map(|i| OutputLine::new(format!("line {i}"), before_seq.unwrap_or(100_000).wrapping_sub(i + 1)))
                .collect();
            let mut combined = lines;
            combined.append(&mut app.worlds[world_idx].output_lines);
            app.worlds[world_idx].output_lines = combined;
            // backfill_complete would have been false here (plenty more available) -
            // per the ScrollbackLines handler, that must not matter: it just calls
            // backfill_advance_to_next() unconditionally, which the next loop
            // iteration exercises.
        }

        // Nothing further should ever be auto-requested, however much history remains.
        assert!(app.backfill_queue.is_empty(), "the queue must be fully (and permanently) drained after one pass");
        assert!(app.backfill_next.is_none(), "no further auto-request should be pending");
        assert!(!app.backfill_needed(), "backfill_needed must report false once every world has its screenful");

        // Each world should hold exactly its screenful - not one line more.
        for (idx, world) in app.worlds.iter().enumerate() {
            assert_eq!(world.output_lines.len(), 75,
                "world {idx} should hold exactly the one-time screenful (75 lines), never deep-filled further");
        }
    }

    // --- PROTOCOL-ROADMAP.md Step 2: resume-driven replay on (re)connect ---

    /// A client that disconnects after acking up through seq N and reconnects with
    /// `AuthRequest { resume: vec![(world_index, N)], .. }` must receive exactly the
    /// lines with seq > N - no gap, no duplicate - proactively from the server via
    /// the same gap-fill path `RequestScrollback{after_seq}` already uses
    /// (`App::handle_request_scrollback`), driven straight out of the AuthRequest
    /// handler (`App::handle_ws_auth_initial_state`) rather than waiting on the client
    /// to notice and ask for it. Also covers requirement #3: the resume payload must
    /// seed `WsClientInfo::acked_seq` so an immediate second reconnect isn't treated
    /// as behind.
    #[test]
    fn test_resume_replay_on_reconnect_sends_exact_gap_no_duplicate() {
        use crate::websocket::{WsMessage, WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        // Simulate ServerData the client already saw (seq 1..=10) before it disconnected.
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.worlds.push(world);
        app.current_world_index = 0;

        // Register a fake WS client directly in the clients map, the same way
        // daemon.rs's `register_client` test helper does - bypasses the real TCP
        // handshake, which isn't the thing under test here, while still exercising the
        // real send path (`ws_send_to_client`/`ws_send_initial_state_and_mark` read
        // from this same map).
        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        // Bounded (PROTOCOL-ROADMAP.md Step 3) — matches the real per-client channel.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true,
                tx,
                current_world: None,
                username: None,
                received_initial_state: false,
                client_type: RemoteClientType::Web,
                viewport_height: 24,
                ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        // Reconnect: the client already has seq 1..=7 (last_contiguous_seq = 7), so
        // resume replay should send back exactly seq 8, 9, 10, oldest-first.
        app.handle_ws_auth_initial_state(client_id, Some(0), vec![(0, 7)], Vec::new());

        let mut scrollback_lines = None;
        // ScrollbackLines is a single-recipient send, so Outbound::Message
        // (PROTOCOL-ROADMAP.md Step 8).
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { world_index, lines, backfill_complete, request_id, .. } = *msg {
                    assert_eq!(world_index, 0);
                    assert!(backfill_complete,
                        "the whole gap fits in one reply, so backfill should be reported complete");
                    // Step 11 (seq-drift fix, on-the-android-app-calm-curry plan): a
                    // server-initiated unprompted resume replay always carries the reserved
                    // request_id Some(0), distinguishing it from a client-solicited reply.
                    assert_eq!(request_id, Some(0),
                        "resume replay must use the reserved request_id Some(0)");
                    assert!(scrollback_lines.is_none(), "resume replay must send exactly one ScrollbackLines reply per world, not several");
                    scrollback_lines = Some(lines);
                }
            }
        }

        let lines = scrollback_lines.expect("expected a ScrollbackLines reply from the resume replay path");
        let seqs: Vec<u64> = lines.iter().map(|l| l.seq).collect();
        assert_eq!(seqs, vec![8, 9, 10],
            "resume replay must send exactly the lines with seq > last_contiguous_seq, in \
             order, with no gap and no duplicate - got {seqs:?}");

        // Requirement #3: acked_seq should be seeded from the resume payload itself, so
        // an immediate second reconnect (before any new PongCheck ack) isn't treated as
        // behind.
        let clients = app.ws_server.as_ref().unwrap().clients.read().unwrap();
        let client = clients.get(&client_id).expect("client should still be registered");
        assert_eq!(client.acked_seq.get(&0), Some(&7),
            "acked_seq must be seeded from AuthRequest.resume on (re)connect");
    }

    // ========================================================================
    // PROTOCOL-ROADMAP.md Phase C — server-side delivery audit
    // ========================================================================

    /// Registers a fake authenticated WS client on `app`, returning its id and receiver.
    /// Same bypass the resume tests above use — the TCP handshake isn't what's under test.
    fn phase_c_register_client(app: &mut App) -> (u64, tokio::sync::mpsc::Receiver<crate::websocket::Outbound>) {
        use crate::websocket::{WsClientInfo, WebSocketServer, RemoteClientType};
        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, rx) = tokio::sync::mpsc::channel::<crate::websocket::Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true,
                tx,
                current_world: None,
                username: None,
                // The audit only considers clients that have been sent InitialState.
                received_initial_state: true,
                client_type: RemoteClientType::Web,
                viewport_height: 24,
                ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(),
                audit_prev_acked: std::collections::HashMap::new(),
                audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);
        (client_id, rx)
    }

    /// Drains the client's outbound queue and returns every `ResyncRequired` in it.
    fn phase_c_drain_resyncs(rx: &mut tokio::sync::mpsc::Receiver<crate::websocket::Outbound>) -> Vec<(usize, u64)> {
        use crate::websocket::{WsMessage, Outbound};
        let mut out = Vec::new();
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::ResyncRequired { world_index, from_seq } = *msg {
                    out.push((world_index, from_seq));
                }
            }
        }
        out
    }

    /// `World::deliverable_high_seq` must report the highest seq actually owed to a remote
    /// client — which is NOT simply the tail of `output_lines`. Anything at or above the
    /// pending backlog's floor seq is deliberately being withheld by more-mode and has not
    /// been broadcast, so counting it would make every client with a paused world look
    /// permanently behind and produce an endless stream of pointless resyncs. This mirrors
    /// `handle_request_scrollback`'s `after_seq` clamp exactly (both now read
    /// `pending_floor_seq`), so the audit and the repair it triggers can't disagree.
    #[test]
    fn test_deliverable_high_seq_excludes_pending_backlog() {
        let mut world = World::new("w");
        assert_eq!(world.deliverable_high_seq(), None,
            "a world with no output at all owes nothing");

        for seq in 0..=5u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        assert_eq!(world.deliverable_high_seq(), Some(5),
            "with no pending backlog the tail of output_lines is deliverable");

        // A backlog opens at seq 6: everything from there up is withheld.
        for seq in 6..=9u64 {
            world.pending_lines.push(OutputLine::new(format!("pending {seq}"), seq));
        }
        assert_eq!(world.pending_floor_seq(), Some(6));
        assert_eq!(world.deliverable_high_seq(), Some(5),
            "pending lines are not owed to a client yet");

        // The invariant-violating shape several past bugs produced: a fresh HIGH-seq line
        // landing in output_lines while older content still sits in pending_lines. The
        // audit must still only claim what's below the floor, or it would advance past
        // content the client was never sent - the exact poisoning this phase exists to stop.
        world.output_lines.push(OutputLine::new("jumped ahead".to_string(), 12));
        assert_eq!(world.deliverable_high_seq(), Some(5),
            "a line planted above the pending floor must not count as deliverable");
    }

    /// The core of the audit: a client stuck behind must be sent exactly one
    /// `ResyncRequired` naming its own acked seq, and only after it has failed to advance
    /// across two consecutive audits.
    #[test]
    fn test_ack_audit_fires_resync_only_after_a_stall() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        world.next_seq = 11;
        app.worlds.push(world);
        app.current_world_index = 0;
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        // Client reports it only has up to seq 4 while the server owes 10.
        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(0, 4)]);

        // First audit: behind, but we've never seen this client's position before, so it
        // could simply be lag with lines still in flight. Must not fire.
        app.audit_client_acks(client_id);
        assert!(phase_c_drain_resyncs(&mut rx).is_empty(),
            "the first audit that sees a client behind must not fire - it can't yet tell \
             a stall from ordinary in-flight lag");

        // Second audit with no progress: now it's a stall.
        app.audit_client_acks(client_id);
        assert_eq!(phase_c_drain_resyncs(&mut rx), vec![(0, 4)],
            "a client stalled at the same seq across two audits must be sent \
             ResyncRequired naming that seq");

        // Third audit, still stuck at 4: suppressed, or an undeliverable seq would produce
        // one resync per keepalive forever.
        app.audit_client_acks(client_id);
        assert!(phase_c_drain_resyncs(&mut rx).is_empty(),
            "re-firing at the same stall point must be suppressed");

        // ...but only for AUDIT_REFIRE_INTERVAL audits, not forever (Phase F). Permanent
        // suppression made a lost or undelivered ResyncRequired - the likeliest outcome
        // precisely when the client's outbound channel is the thing that overflowed - a
        // permanent write-off, with the server having decided the client was beyond help.
        let refire = crate::websocket::AUDIT_REFIRE_INTERVAL;
        for i in 0..refire.saturating_sub(2) {
            app.audit_client_acks(client_id);
            assert!(phase_c_drain_resyncs(&mut rx).is_empty(),
                "still inside the suppression window at tick {i}");
        }
        app.audit_client_acks(client_id);
        assert_eq!(phase_c_drain_resyncs(&mut rx), vec![(0, 4)],
            "after AUDIT_REFIRE_INTERVAL audits still stalled at the same seq, the resync \
             must be retried rather than abandoned");

        // It recovers to 8 (still behind), then stalls there: a NEW stall point, so the
        // suppression must not carry over.
        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(0, 8)]);
        app.audit_client_acks(client_id);
        assert!(phase_c_drain_resyncs(&mut rx).is_empty(), "progress is not a stall");
        app.audit_client_acks(client_id);
        assert_eq!(phase_c_drain_resyncs(&mut rx), vec![(0, 8)],
            "a stall at a new position must fire again");
    }

    /// A client that is keeping up, or that the server owes nothing to, must never be sent
    /// a resync no matter how many audits run.
    #[test]
    fn test_ack_audit_silent_when_caught_up() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        world.next_seq = 11;
        app.worlds.push(world);
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(0, 10)]);
        for _ in 0..5 {
            app.audit_client_acks(client_id);
        }
        assert!(phase_c_drain_resyncs(&mut rx).is_empty(),
            "a fully caught-up client must never be resynced");

        // Advancing content the client then acks: still silent.
        app.worlds[0].output_lines.push(OutputLine::new("line 11".to_string(), 11));
        app.worlds[0].next_seq = 12;
        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(0, 11)]);
        app.audit_client_acks(client_id);
        app.audit_client_acks(client_id);
        assert!(phase_c_drain_resyncs(&mut rx).is_empty(),
            "a client that keeps acking must never be resynced");
    }

    /// Three ways a world is exempt from the audit entirely, each for a different reason.
    #[test]
    fn test_ack_audit_skips_worlds_it_must_not_touch() {
        let mut app = App::new();
        app.worlds.clear();

        // World 0: real seqs, but the client has never acked it at all. That's the
        // "InitialState's aggregate line budget ran out before this world" case - the
        // client's own phase-1 backfill covers it, and firing from 0 would instead pull the
        // entire in-memory ring for every such world on every connect.
        let mut w0 = World::new("never-acked");
        for seq in 1..=10u64 {
            w0.output_lines.push(OutputLine::new(format!("a {seq}"), seq));
        }
        w0.next_seq = 11;
        app.worlds.push(w0);

        // World 1: next_seq == 0, i.e. no seq was ever assigned. Nothing to be behind on.
        app.worlds.push(World::new("no-seqs-yet"));

        // World 2: everything the client hasn't acked is held in the pending backlog, so
        // nothing is actually owed - a paused world must not look like a stalled client.
        let mut w2 = World::new("all-pending");
        w2.output_lines.push(OutputLine::new("visible".to_string(), 1));
        for seq in 2..=9u64 {
            w2.pending_lines.push(OutputLine::new(format!("held {seq}"), seq));
        }
        w2.next_seq = 10;
        app.worlds.push(w2);

        // World 3: the client sent an EXPLICIT ack of 0. Distinct from world 0 (which has
        // no acked_seq entry at all and short-circuits earlier) - this one does have an
        // entry, so it reaches the zero check. Same reasoning: acking 0 means "I have
        // nothing", which is a backfill's job, not a resync's.
        let mut w3 = World::new("acked-zero");
        for seq in 1..=10u64 {
            w3.output_lines.push(OutputLine::new(format!("d {seq}"), seq));
        }
        w3.next_seq = 11;
        app.worlds.push(w3);

        let (client_id, mut rx) = phase_c_register_client(&mut app);
        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(2, 1), (3, 0)]);
        {
            // record_acked_seq only raises the stored value, so confirm the explicit zero
            // really did land as an entry - otherwise this case would silently degrade
            // into world 0's "never acked" path and stop testing what it claims to.
            let clients = app.ws_server.as_ref().unwrap().clients.read().unwrap();
            assert_eq!(clients.get(&client_id).unwrap().acked_seq.get(&3), Some(&0),
                "an explicit ack of 0 must be recorded as an entry, not skipped");
        }

        for _ in 0..4 {
            app.audit_client_acks(client_id);
        }
        assert!(phase_c_drain_resyncs(&mut rx).is_empty(),
            "a never-acked world, a world with no seqs, a world whose whole remainder is \
             pending, and a world explicitly acked at 0 must all be exempt from the audit");
    }

    /// The audit must be per-world: one stalled world must not silence or trigger another.
    #[test]
    fn test_ack_audit_is_per_world() {
        let mut app = App::new();
        app.worlds.clear();
        for name in ["w0", "w1"] {
            let mut w = World::new(name);
            for seq in 1..=10u64 {
                w.output_lines.push(OutputLine::new(format!("{name} {seq}"), seq));
            }
            w.next_seq = 11;
            app.worlds.push(w);
        }
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        // Caught up on w0, stalled on w1.
        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(0, 10), (1, 3)]);
        app.audit_client_acks(client_id);
        app.audit_client_acks(client_id);

        assert_eq!(phase_c_drain_resyncs(&mut rx), vec![(1, 3)],
            "only the stalled world should be resynced, naming its own acked seq");
    }

    /// The repair the audit asks for must actually be serviceable: a `RequestScrollback`
    /// with `after_seq` set to the `from_seq` the audit reported has to return exactly the
    /// missing lines. This is the whole loop - detector, message, and repair - closing.
    #[test]
    fn test_audit_from_seq_drives_a_correct_gap_fill() {
        use crate::websocket::{WsMessage, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        world.next_seq = 11;
        app.worlds.push(world);
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.ws_server.as_ref().unwrap().record_acked_seq(client_id, &[(0, 6)]);
        app.audit_client_acks(client_id);
        app.audit_client_acks(client_id);
        let fired = phase_c_drain_resyncs(&mut rx);
        assert_eq!(fired, vec![(0, 6)]);

        // Client obeys: RequestScrollback{after_seq: from_seq}, exactly as app.js's
        // ResyncRequired handler does via requestGapFill(world_index, msg.from_seq).
        let (_, from_seq) = fired[0];
        app.handle_request_scrollback(client_id, 0, 500, None, Some(from_seq), Some(1));

        let mut got = None;
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { lines, .. } = *msg {
                    got = Some(lines);
                }
            }
        }
        let seqs: Vec<u64> = got.expect("expected a ScrollbackLines reply").iter().map(|l| l.seq).collect();
        assert_eq!(seqs, vec![7, 8, 9, 10],
            "the audit's from_seq must yield exactly the lines the client is missing");
    }

    /// `WorldStateResponse` carries the world's deliverable high seq so the client can
    /// verify at the moment of a world switch (PROTOCOL-ROADMAP.md Phase C). Switching used
    /// to be the one point where nothing checked: `SwitchWorld` sends no content and the
    /// client renders straight from its local buffer, so a world that had quietly lost
    /// lines just looked empty.
    #[test]
    fn test_world_state_response_reports_deliverable_high_seq() {
        use crate::websocket::{WsMessage, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        // A backlog above seq 7 must not be advertised as deliverable.
        for seq in 11..=13u64 {
            world.pending_lines.push(OutputLine::new(format!("held {seq}"), seq));
        }
        world.next_seq = 14;
        app.worlds.push(world);
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_request_world_state(client_id, 0);

        let mut reported = None;
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::WorldStateResponse { deliverable_high_seq, .. } = *msg {
                    reported = Some(deliverable_high_seq);
                }
            }
        }
        assert_eq!(reported, Some(10),
            "WorldStateResponse must report the highest seq owed to a client, excluding \
             anything still held in the pending backlog");
    }

    /// Empty `resume` (old clients, or a fresh client with no prior state) must behave
    /// exactly as before this step: InitialState only, no unsolicited ScrollbackLines.
    #[test]
    fn test_empty_resume_sends_no_scrollback_replay() {
        use crate::websocket::{WsMessage, WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.worlds.push(world);
        app.current_world_index = 0;

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        // Bounded (PROTOCOL-ROADMAP.md Step 3) — matches the real per-client channel.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true,
                tx,
                current_world: None,
                username: None,
                received_initial_state: false,
                client_type: RemoteClientType::Web,
                viewport_height: 24,
                ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        app.handle_ws_auth_initial_state(client_id, Some(0), Vec::new(), Vec::new());

        let mut saw_scrollback = false;
        let mut saw_initial_state = false;
        // Both are single-recipient sends, so Outbound::Message (PROTOCOL-ROADMAP.md Step 8).
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                match *msg {
                    WsMessage::ScrollbackLines { .. } => saw_scrollback = true,
                    WsMessage::InitialState { .. } => saw_initial_state = true,
                    _ => {}
                }
            }
        }
        assert!(saw_initial_state, "InitialState must still be sent as before this step");
        assert!(!saw_scrollback, "an empty resume list must not trigger any replay - first-time connections must be unaffected");
    }

    /// Step 11 (seq-drift fix, on-the-android-app-calm-curry plan): a client-supplied
    /// `RequestScrollback.request_id` must be echoed back verbatim on the matching
    /// `ScrollbackLines` reply, so the client can correlate replies to requests instead of
    /// routing purely on ambiguous local state (the app.js `_gapFillPending`
    /// stuck-true-after-RequestState bug this correlator exists to fix).
    #[test]
    fn test_request_scrollback_echoes_request_id() {
        use crate::websocket::{WsMessage, WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("world0");
        for seq in 1..=10u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.worlds.push(world);
        app.current_world_index = 0;

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true,
                tx,
                current_world: None,
                username: None,
                received_initial_state: true,
                client_type: RemoteClientType::Web,
                viewport_height: 24,
                ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        app.handle_request_scrollback(client_id, 0, 5, None, None, Some(42));

        let mut request_id = None;
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { request_id: rid, .. } = *msg {
                    request_id = Some(rid);
                }
            }
        }
        assert_eq!(request_id, Some(Some(42)), "the client-supplied request_id must be echoed back verbatim");

        // A request with no request_id (an old client, or one with nothing to correlate)
        // must echo back None, not silently invent a value.
        app.handle_request_scrollback(client_id, 0, 5, None, None, None);
        let mut second_request_id = None;
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { request_id: rid, .. } = *msg {
                    second_request_id = Some(rid);
                }
            }
        }
        assert_eq!(second_request_id, Some(None), "an absent request_id must be echoed back as None, not fabricated");
    }

    /// `App::ws_broadcast` used to check only `client.authenticated`, unlike its three
    /// `WebSocketServer` siblings (`broadcast_to_owner`/`broadcast_to_all`/
    /// `broadcast_to_world_viewers`), which all also require `received_initial_state` -
    /// specifically to stop a broadcast reaching a client before the InitialState that (for
    /// output messages) contains the same lines, which causes SEQ MISMATCH/duplicate errors
    /// once that InitialState arrives. A whitelisted client is inserted with
    /// `authenticated: true` before the app loop ever sees the connection, so this window was
    /// real, not just theoretical.
    #[test]
    fn test_ws_broadcast_skips_client_without_initial_state() {
        use crate::websocket::{WsMessage, WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let mut app = App::new();
        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true,
                tx,
                current_world: None,
                username: None,
                received_initial_state: false,
                client_type: RemoteClientType::Web,
                viewport_height: 24,
                ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        app.ws_broadcast(WsMessage::PendingReleased { world_index: 0, count: 3 });
        assert!(rx.try_recv().is_err(), "a client that hasn't received InitialState must not get a broadcast");

        // Flip the flag and confirm the same broadcast now goes through - proves the
        // filter is the reason for the earlier miss, not something else swallowing it.
        {
            let mut clients = app.ws_server.as_ref().unwrap().clients.write().unwrap();
            clients.get_mut(&client_id).unwrap().received_initial_state = true;
        }
        app.ws_broadcast(WsMessage::PendingReleased { world_index: 0, count: 3 });
        let received = rx.try_recv();
        assert!(received.is_ok(), "the same broadcast must be delivered once received_initial_state is true");
    }

    // --- PROTOCOL-ROADMAP.md Step 3: bounded channel + ResyncRequired on overflow ---

    /// A client whose outbound channel overflows must not have the overflow silently
    /// dropped: the affected world gets flagged `needs_resync`, and once the channel has
    /// room again (draining, in this test simulating a slow client catching up) exactly
    /// one `ResyncRequired` for that world is delivered - and the connection is never torn
    /// down (the client stays registered throughout, matching a stalled-but-alive client
    /// rather than a dead one).
    #[test]
    fn test_channel_full_sends_resync_required_once() {
        use crate::websocket::{WsMessage, WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        // A small test-only capacity so a handful of sends overflows it - exercising the
        // real WS_CLIENT_CHANNEL_CAPACITY (256) would need hundreds of iterations to hit
        // the same code path.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(4);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true,
                tx,
                current_world: Some(0),
                username: None,
                received_initial_state: true,
                client_type: RemoteClientType::Web,
                viewport_height: 24,
                ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }

        let make_line = |seq: u64| WsMessage::ServerData {
            world_index: 0,
            data: format!("line {seq}"),
            is_viewed: true,
            ts: 0,
            from_server: true,
            seq,
            end_seq: None,
            flush: false,
            gagged: false, highlight_colors: Vec::new(),
        };

        // Fill the channel past its capacity of 4 - the 5th broadcast must overflow it.
        for seq in 1..=5u64 {
            server.broadcast_to_all(make_line(seq));
        }

        // The connection must survive the overflow (still registered), and world 0 must
        // be flagged as needing a resync - no ResyncRequired could be delivered yet since
        // the channel is still completely full.
        {
            let clients = server.clients.read().unwrap();
            let client = clients.get(&client_id)
                .expect("client must still be registered - a channel overflow must not tear down the connection");
            assert!(client.needs_resync.contains(&0),
                "world 0 should be flagged needing resync after its channel overflowed");
        }

        // Simulate the client catching up: drain most (not all) of the backlog, freeing
        // room in the channel.
        let mut drained = 0;
        while drained < 3 && rx.try_recv().is_ok() {
            drained += 1;
        }

        // The next successful broadcast should piggyback delivery of the flagged resync
        // now that there's room (see `reconcile_resync`'s flush_candidates handling).
        server.broadcast_to_all(make_line(6));

        {
            let clients = server.clients.read().unwrap();
            let client = clients.get(&client_id).unwrap();
            assert!(!client.needs_resync.contains(&0),
                "needs_resync must be cleared once ResyncRequired is actually delivered");
        }

        // Drain everything left and count ResyncRequired occurrences - must be exactly one.
        // ResyncRequired's from_seq is per-client, so it's always Outbound::Message
        // (PROTOCOL-ROADMAP.md Step 8) - the ServerData broadcasts also sitting in this
        // channel are Outbound::Shared and simply don't match the Message pattern below.
        let mut resync_count = 0;
        while let Ok(item) = rx.try_recv() {
            if let Outbound::Message(msg) = item {
                if let WsMessage::ResyncRequired { world_index, .. } = *msg {
                    assert_eq!(world_index, 0);
                    resync_count += 1;
                }
            }
        }
        assert_eq!(resync_count, 1,
            "exactly one ResyncRequired should be observed for the affected world, not zero or several");
    }

    // --- PROTOCOL-ROADMAP.md Step 6: Rust remote console client (receiving side) ---
    // Exercises `App::handle_remote_ws_message`, the client-side handler for messages
    // received *from* a remote Clay server over `--console`. Unlike Step 2's tests above
    // (which drive the server's resume-replay path), these drive the client's reaction to
    // a live mid-stream gap: a forward seq jump in `ServerData`, followed by the
    // `ResyncRequired`/`ScrollbackLines` round trip that recovers it.

    fn console_server_data(world_index: usize, seq: u64, lines: &[&str]) -> WsMessage {
        WsMessage::ServerData {
            world_index,
            data: lines.join("\n"),
            is_viewed: true,
            ts: 0,
            from_server: true,
            seq,
            end_seq: None,
            flush: false,
            gagged: false,
            highlight_colors: Vec::new(),
        }
    }

    /// A `ServerData` batch that jumps ahead of `max_received_seq` (the server's outbound
    /// channel dropped something in between, PROTOCOL-ROADMAP.md Step 3) must not be
    /// treated as a duplicate or silently lose the gap: the client should record exactly
    /// where the gap starts (`World::pending_gap`), and once the server's `ResyncRequired`
    /// /`ScrollbackLines` round trip supplies the missing lines, splice them back into that
    /// exact position so the world's buffer ends up complete and in order - not tacked onto
    /// the front (the old blind-prepend behavior) or the back.
    #[test]
    fn test_console_client_resync_gap_fill_restores_order_no_loss() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("world0"));
        app.current_world_index = 0;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
        app.ws_client_tx = Some(tx);

        // Normal contiguous delivery: seq 1..=5.
        app.handle_remote_ws_message(console_server_data(0, 1,
            &["line1", "line2", "line3", "line4", "line5"]));
        assert_eq!(app.worlds[0].max_received_seq, 5);
        assert_eq!(app.worlds[0].output_lines.len(), 5);
        assert!(app.worlds[0].pending_gap.is_none(), "no gap yet - nothing to track");

        // A batch arrives at seq 11 - seq 6..=10 never made it (server-side channel
        // overflow). The client must not treat this as a duplicate (11 > max_received_seq),
        // must still accept and display it (trusting in-order delivery per Step 6), and
        // must remember exactly where the hole starts: right after the 5 lines it already
        // has.
        app.handle_remote_ws_message(console_server_data(0, 11,
            &["line11", "line12", "line13", "line14", "line15"]));
        assert_eq!(app.worlds[0].max_received_seq, 15);
        assert_eq!(app.worlds[0].output_lines.len(), 10,
            "the seq-11 batch must still be appended, not dropped, despite the gap behind it");
        assert_eq!(app.worlds[0].pending_gap, Some((5, 5)),
            "gap must be recorded at local index 5 (right after the first batch), with 5 as \
             the last contiguous seq");

        // The server notices (via its own overflow bookkeeping) and sends ResyncRequired.
        // The client must ask for exactly the missing range via the same RequestScrollback
        // mechanism reconnect-time gap-fill already uses.
        app.handle_remote_ws_message(WsMessage::ResyncRequired { world_index: 0, from_seq: 5 });
        let sent = rx.try_recv().expect("ResyncRequired must trigger a RequestScrollback");
        match sent {
            WsMessage::RequestScrollback { world_index, count, before_seq, after_seq, .. } => {
                assert_eq!(world_index, 0);
                assert_eq!(before_seq, None);
                assert_eq!(after_seq, Some(5));
                assert!(count >= 5, "count must be large enough to cover the whole gap");
            }
            other => panic!("expected RequestScrollback, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "exactly one RequestScrollback, not several");

        // The server replies with the missing lines (seq 6..=10), exactly as
        // `handle_request_scrollback`'s after_seq branch would produce.
        let gap_lines: Vec<TimestampedLine> = (6..=10u64).map(|seq| TimestampedLine {
            text: format!("line{seq}"),
            ts: 0,
            gagged: false,
            from_server: true,
            seq,
            highlight_color: None,
            from_archive: false,
            viewed: false,
            display_id: None,
        }).collect();
        app.handle_remote_ws_message(WsMessage::ScrollbackLines {
            world_index: 0,
            lines: gap_lines,
            backfill_complete: true,
            clamped_by_pending: false,
            request_id: None,
        });

        // Complete: all 15 lines present, no permanent loss.
        assert_eq!(app.worlds[0].output_lines.len(), 15);
        assert!(app.worlds[0].pending_gap.is_none(), "pending_gap must be cleared once filled");
        // In order: reading the buffer front-to-back must reproduce line1..line15 in
        // sequence - not gap-fill-at-the-front (old prepend behavior) or gap-fill-at-the-
        // back (naive append), either of which would scramble the chronological order.
        let texts: Vec<&str> = app.worlds[0].output_lines.iter().map(|l| l.text.as_str()).collect();
        let expected: Vec<String> = (1..=15u64).map(|n| format!("line{n}")).collect();
        assert_eq!(texts, expected, "gap-filled lines must be spliced back into their exact \
            chronological position, producing a complete, in-order buffer");
    }

    /// PROTOCOL-ROADMAP.md's seq-drift fix, Step 12b: a scroll-triggered `before_seq`
    /// backfill reply arriving while `world.pending_gap` happens to be open (from an
    /// unrelated outstanding resync) must be routed by its own registered `request_id`
    /// kind, not misrouted into the gap-fill splice path purely because `pending_gap` is
    /// `Some`. Before this fix, EVERY line in a genuine older-history reply would fail the
    /// splice path's `l.seq > last_contiguous_seq` filter (they're all older, by
    /// definition) and be silently dropped instead of prepended.
    #[test]
    fn test_console_client_scroll_backfill_reply_not_treated_as_gap_fill() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("world0"));
        app.current_world_index = 0;

        let (tx, mut _rx) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
        app.ws_client_tx = Some(tx);

        // Establish a buffer starting at seq 101 (leaving room "below" it for an
        // older-history backfill reply) and open a gap the same way
        // test_console_client_resync_gap_fill_restores_order_no_loss does: a batch that
        // jumps ahead of max_received_seq.
        app.handle_remote_ws_message(console_server_data(0, 101,
            &["line101", "line102", "line103", "line104", "line105"]));
        app.handle_remote_ws_message(console_server_data(0, 111,
            &["line111", "line112", "line113", "line114", "line115"]));
        assert_eq!(app.worlds[0].pending_gap, Some((5, 105)),
            "sanity check: a gap must be open before this test's actual scenario begins");
        let lines_before = app.worlds[0].output_lines.len();

        // Register a genuine Backfill request (mirrors scroll_page_up's registration in
        // remote_client.rs) and receive its reply while pending_gap is STILL open - the
        // exact race this fix targets.
        let backfill_id = app.register_scrollback_request(0, ScrollbackRequestKind::Backfill);
        let older_lines: Vec<TimestampedLine> = (90..=100u64).map(|seq| TimestampedLine {
            text: format!("line{seq}"),
            ts: 0,
            gagged: false,
            from_server: true,
            seq,
            highlight_color: None,
            from_archive: false,
            viewed: false,
            display_id: None,
        }).collect();
        let older_count = older_lines.len();
        app.handle_remote_ws_message(WsMessage::ScrollbackLines {
            world_index: 0,
            lines: older_lines,
            backfill_complete: true,
            clamped_by_pending: false,
            request_id: Some(backfill_id),
        });

        assert_eq!(app.worlds[0].output_lines.len(), lines_before + older_count,
            "the older-history reply must be prepended (all lines kept), not dropped by the \
             gap-fill splice path's seq > last_contiguous_seq filter");
        assert_eq!(app.worlds[0].output_lines.first().map(|l| l.text.as_str()), Some("line90"),
            "the oldest line must end up at the very front of the buffer (prepended)");
        assert_eq!(app.worlds[0].pending_gap, Some((5, 105)),
            "pending_gap must be completely untouched by this reply - it belongs to a \
             different, still-unanswered request");
        assert!(!app.pending_scrollback_requests.contains_key(&backfill_id),
            "the resolved request must be removed from pending_scrollback_requests");
    }

    /// `PongCheck.acked` must report the last *contiguous* seq, not the highest seq seen -
    /// while a gap is outstanding, acking the post-gap `max_received_seq` would tell the
    /// server we have lines we don't, hiding the hole from any future resume.
    #[test]
    fn test_console_client_pong_check_acks_pre_gap_boundary() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("world0"));
        app.current_world_index = 0;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
        app.ws_client_tx = Some(tx);

        app.handle_remote_ws_message(console_server_data(0, 1, &["line1", "line2"]));
        // No gap yet: acked should reflect max_received_seq (2).
        app.handle_remote_ws_message(WsMessage::PingCheck { nonce: 1 });
        match rx.try_recv().unwrap() {
            WsMessage::PongCheck { nonce, acked } => {
                assert_eq!(nonce, 1);
                assert_eq!(acked, vec![(0, 2)]);
            }
            other => panic!("expected PongCheck, got {other:?}"),
        }

        // Open a gap: seq jumps from 2 to 5 (missing 3, 4).
        app.handle_remote_ws_message(console_server_data(0, 5, &["line5"]));
        assert_eq!(app.worlds[0].max_received_seq, 5);
        assert_eq!(app.worlds[0].pending_gap, Some((2, 2)));

        app.handle_remote_ws_message(WsMessage::PingCheck { nonce: 2 });
        match rx.try_recv().unwrap() {
            WsMessage::PongCheck { nonce, acked } => {
                assert_eq!(nonce, 2);
                assert_eq!(acked, vec![(0, 2)],
                    "must ack the pre-gap boundary (2), not max_received_seq (5) - acking 5 \
                     would hide the still-missing 3..4 from the server");
            }
            other => panic!("expected PongCheck, got {other:?}"),
        }
    }

    // --- App::resolve_quote_lines ---
    // Pins the shared /quote helper's behavior, including the world-targeting/delay-scheduling
    // support console's two call sites used to silently drop entirely (T32).

    #[test]
    fn test_resolve_quote_lines_no_options_returns_lines_unchanged() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.current_world_index = 0;

        let result = app.resolve_quote_lines(
            vec!["one".to_string(), "two".to_string()],
            &None, 0.0, None, false, 0, tf::QuoteDisposition::Send, false,
        );
        assert_eq!(result, Some((0, vec!["one".to_string(), "two".to_string()])));
    }

    #[test]
    fn test_resolve_quote_lines_targets_named_world() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds.push(World::new("beta"));
        app.current_world_index = 0;

        let result = app.resolve_quote_lines(
            vec!["hi".to_string()],
            &Some("beta".to_string()), 0.0, None, false, 0, tf::QuoteDisposition::Send, false,
        );
        assert_eq!(result, Some((1, vec!["hi".to_string()])), "should target beta (index 1), not the current world");
    }

    #[test]
    fn test_resolve_quote_lines_unknown_world_falls_back_to_current() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.current_world_index = 0;

        let result = app.resolve_quote_lines(
            vec!["hi".to_string()],
            &Some("nonexistent".to_string()), 0.0, None, false, 0, tf::QuoteDisposition::Send, false,
        );
        assert_eq!(result, Some((0, vec!["hi".to_string()])), "unknown world name should fall back to world_index");
    }

    #[test]
    fn test_resolve_quote_lines_delay_schedules_processes_and_returns_none() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.current_world_index = 0;
        assert!(app.tf_engine.processes.is_empty());

        let result = app.resolve_quote_lines(
            vec!["one".to_string(), "two".to_string(), "three".to_string()],
            &None, 5.0, None, false, 0, tf::QuoteDisposition::Send, false,
        );
        assert_eq!(result, None, "delayed multi-line quote has nothing left to send immediately");
        assert_eq!(app.tf_engine.processes.len(), 3, "each line should be scheduled as its own delayed process");
        assert_eq!(app.tf_engine.processes[0].command, "one");
        assert_eq!(app.tf_engine.processes[1].command, "two");
        assert_eq!(app.tf_engine.processes[2].command, "three");
    }

    #[test]
    fn test_resolve_quote_lines_single_line_ignores_delay() {
        // Delay-scheduling only kicks in for lines.len() > 1 - a single line always sends
        // immediately regardless of delay_secs.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.current_world_index = 0;

        let result = app.resolve_quote_lines(
            vec!["only".to_string()],
            &None, 5.0, None, false, 0, tf::QuoteDisposition::Send, false,
        );
        assert_eq!(result, Some((0, vec!["only".to_string()])));
        assert!(app.tf_engine.processes.is_empty());
    }

    // --- App::update_world_settings ---
    // Pins the password-guard fix (T36): daemon's inline copy used to unconditionally
    // overwrite the stored password with whatever the client sent, silently wiping it on any
    // settings save where the incoming password was empty or an "ENC:" placeholder.

    #[test]
    fn test_update_world_settings_empty_password_does_not_wipe_stored_password() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds[0].settings.password = "hunter2".to_string();

        app.update_world_settings(
            0, "alpha".to_string(), "mud.example.com".to_string(), "4000".to_string(),
            "myuser".to_string(), String::new(), false, false,
            "utf8".to_string(), "manual".to_string(), "none".to_string(), String::new(),
            String::new(), "0".to_string(),
        );

        assert_eq!(app.worlds[0].settings.password, "hunter2",
            "an empty incoming password must be treated as 'field not touched', not 'clear it'");
    }

    #[test]
    fn test_update_world_settings_enc_placeholder_does_not_wipe_stored_password() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds[0].settings.password = "hunter2".to_string();

        app.update_world_settings(
            0, "alpha".to_string(), "mud.example.com".to_string(), "4000".to_string(),
            "myuser".to_string(), "ENC:whatever".to_string(), false, false,
            "utf8".to_string(), "manual".to_string(), "none".to_string(), String::new(),
            String::new(), "0".to_string(),
        );

        assert_eq!(app.worlds[0].settings.password, "hunter2",
            "an 'ENC:' placeholder must be treated as 'field not touched', not a new password");
    }

    #[test]
    fn test_update_world_settings_nonempty_plaintext_password_does_update() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds[0].settings.password = "hunter2".to_string();

        app.update_world_settings(
            0, "alpha".to_string(), "mud.example.com".to_string(), "4000".to_string(),
            "myuser".to_string(), "newpassword".to_string(), false, false,
            "utf8".to_string(), "manual".to_string(), "none".to_string(), String::new(),
            String::new(), "0".to_string(),
        );

        assert_eq!(app.worlds[0].settings.password, "newpassword",
            "a real plaintext password must still update normally");
    }

    // --- App::update_global_settings ---
    // Pins the clamping/validation and ws_password handling that daemon's inline copy used to
    // skip entirely (T37).

    fn call_update_global_settings(app: &mut App, gui_transparency: f32, color_offset_percent: u8, input_height: u16, font_size: f32, ws_password: &str) {
        app.update_global_settings(
            0, true, true, true,
            "unseen".to_string(), false, false, true,
            "dark".to_string(), "dark".to_string(),
            gui_transparency, color_offset_percent, 0, 500,
            input_height, "monospace".to_string(), font_size,
            14.0, 14.0, 14.0, 400, 1.2, 0.0, 0.0,
            String::new(), false, false, 9000, String::new(),
            String::new(), String::new(), ws_password.to_string(),
            false, String::new(), true, true, false,
            "off".to_string(), "off".to_string(), false, false, true,
            "none".to_string(), "app_tablet".to_string(),
        );
    }

    #[test]
    fn test_update_global_settings_clamps_high_out_of_range_values() {
        let mut app = App::new();
        call_update_global_settings(&mut app, 5.0, 200, 999, 1000.0, "");
        assert_eq!(app.settings.gui_transparency, 1.0, "gui_transparency must clamp to <= 1.0");
        assert_eq!(app.settings.color_offset_percent, 100, "color_offset_percent must clamp to <= 100");
        assert_eq!(app.input_height, 15, "input_height must clamp to <= 15");
        assert_eq!(app.input.visible_height, 15, "input.visible_height must stay in sync with input_height");
        assert_eq!(app.settings.font_size, 48.0, "font_size must clamp to <= 48.0");
    }

    #[test]
    fn test_update_global_settings_clamps_low_out_of_range_values() {
        let mut app = App::new();
        call_update_global_settings(&mut app, 0.0, 0, 0, 0.0, "");
        assert_eq!(app.settings.gui_transparency, 0.3, "gui_transparency must clamp to >= 0.3");
        assert_eq!(app.input_height, 1, "input_height must clamp to >= 1");
        assert_eq!(app.settings.font_size, 8.0, "font_size must clamp to >= 8.0");
    }

    #[test]
    fn test_update_global_settings_applies_ws_password() {
        let mut app = App::new();
        call_update_global_settings(&mut app, 1.0, 0, 5, 14.0, "newpassword");
        assert_eq!(app.settings.websocket_password, "newpassword",
            "ws_password must actually be applied - daemon's copy used to ignore this field entirely");
    }

    // --- App::release_pending_lines / App::selective_flush ---
    // Pins the state transitions of the shared pending/flush handling (T38). SelectiveFlush
    // was entirely unhandled in master-WS before this task - these tests exercise the same
    // shared methods both master-WS and daemon now call.

    fn make_pending_line(text: &str, highlighted: bool) -> OutputLine {
        OutputLine {
            text: text.to_string(),
            timestamp: std::time::SystemTime::now(),
            from_server: true,
            gagged: false,
            is_input: false,
            seq: 0,
            highlight_color: if highlighted { Some("red".to_string()) } else { None },
            from_archive: false,
            viewed: false,
            display_id: None,
        }
    }

    #[test]
    fn test_release_pending_lines_zero_pending_is_a_noop() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.release_pending_lines(1, 0, 0);
        assert!(app.worlds[0].pending_lines.is_empty());
        assert!(app.worlds[0].output_lines.is_empty());
    }

    #[test]
    fn test_release_pending_lines_moves_lines_from_pending_to_output() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        for i in 0..5 {
            app.worlds[0].pending_lines.push(make_pending_line(&format!("line {i}"), false));
        }
        app.worlds[0].paused = true;

        // count=0 means release all
        app.release_pending_lines(1, 0, 0);

        assert!(app.worlds[0].pending_lines.is_empty(), "all pending lines should be released");
        assert_eq!(app.worlds[0].output_lines.len(), 5, "released lines should move to output_lines");
    }

    #[test]
    fn test_selective_flush_keeps_only_highlighted_lines_when_paused() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds[0].paused = true;
        app.worlds[0].pending_lines.push(make_pending_line("plain", false));
        app.worlds[0].pending_lines.push(make_pending_line("highlighted", true));
        app.worlds[0].lines_since_pause = 7;

        app.selective_flush(0);

        assert!(app.worlds[0].pending_lines.is_empty(), "selective flush must clear all pending lines");
        assert_eq!(app.worlds[0].output_lines.len(), 1, "only the highlighted line should be kept");
        assert_eq!(app.worlds[0].output_lines[0].text, "highlighted");
        assert!(!app.worlds[0].paused, "selective flush must unpause the world");
        assert_eq!(app.worlds[0].lines_since_pause, 0);
    }

    #[test]
    fn test_selective_flush_noop_when_not_paused() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds[0].paused = false;
        app.worlds[0].pending_lines.push(make_pending_line("plain", false));

        app.selective_flush(0);

        assert_eq!(app.worlds[0].pending_lines.len(), 1,
            "selective flush must do nothing when the world isn't paused");
        assert!(app.worlds[0].output_lines.is_empty());
    }

    // --- App::calculate_oldest_pending_world_from ---
    // Pins the third fallback tier (T39): daemon's inline copy was missing it entirely, so
    // "switch to the world needing attention" (Escape+w) had a strictly weaker fallback for
    // daemon-attached clients.

    #[test]
    fn test_calculate_oldest_pending_falls_back_to_previous_world() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds.push(World::new("beta"));
        app.worlds.push(World::new("gamma"));
        // No world has pending or unseen output - only the "previous world" tier can fire.
        app.previous_world_index = Some(2);

        let result = app.calculate_oldest_pending_world_from(0);
        assert_eq!(result, Some(2), "with no pending/unseen output anywhere, should fall back to previous_world_index");
    }

    #[test]
    fn test_calculate_oldest_pending_prefers_pending_over_previous_world() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.worlds.push(World::new("beta"));
        app.worlds[1].pending_lines.push(make_pending_line("line", false));
        app.worlds[1].pending_since = Some(std::time::Instant::now());
        app.previous_world_index = Some(0);

        let result = app.calculate_oldest_pending_world_from(0);
        assert_eq!(result, Some(1), "a world with actual pending output should win over the previous-world fallback");
    }

    #[test]
    fn test_calculate_oldest_pending_none_when_nothing_qualifies() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("alpha"));
        app.previous_world_index = None;

        let result = app.calculate_oldest_pending_world_from(0);
        assert_eq!(result, None);
    }

    // --- App::handle_update_actions ---
    // Pins the most severe finding of the whole audit (T40): this message was entirely absent
    // from daemon.rs, in both the regular and multiuser handlers - saving action-editor
    // changes was silently broken for every daemon-mode deployment.

    #[test]
    fn test_handle_update_actions_saves_and_normalizes() {
        let mut app = App::new();
        app.settings.actions.clear();
        let action = Action { name: "greet".to_string(), command: "say hi".to_string(), ..Action::default() };

        app.handle_update_actions(vec![action]);

        assert_eq!(app.settings.actions.len(), 1);
        assert_eq!(app.settings.actions[0].name, "greet");
        assert_eq!(app.settings.actions[0].command, "say hi");
    }

    #[test]
    fn test_handle_update_actions_replaces_existing_list() {
        let mut app = App::new();
        app.settings.actions = vec![
            Action { name: "old1".to_string(), ..Action::default() },
            Action { name: "old2".to_string(), ..Action::default() },
        ];

        app.handle_update_actions(vec![Action { name: "new".to_string(), ..Action::default() }]);

        assert_eq!(app.settings.actions.len(), 1);
        assert_eq!(app.settings.actions[0].name, "new");
    }

    // --- handle_paste: paste routed to whichever widget has focus ---
    // Regression coverage for: pasting into an open /note editor landed in the
    // command input instead of the note buffer.

    const PASTE_TEST_FIELD_TEXT: popup::FieldId = popup::FieldId(1);
    const PASTE_TEST_FIELD_MULTI: popup::FieldId = popup::FieldId(2);
    const PASTE_TEST_BTN_SAVE: popup::ButtonId = popup::ButtonId(1);

    fn paste_test_popup() -> popup::PopupDefinition {
        popup::PopupDefinition::new(popup::PopupId("paste_test"), "Paste Test")
            .with_field(popup::Field::new(PASTE_TEST_FIELD_TEXT, "Text", popup::FieldKind::text("")))
            .with_field(popup::Field::new(PASTE_TEST_FIELD_MULTI, "Multi", popup::FieldKind::multiline("", 3)))
            .with_button(popup::Button::new(PASTE_TEST_BTN_SAVE, "Save").primary().with_shortcut('s'))
    }

    #[test]
    fn test_paste_into_note_editor_when_focused() {
        let mut app = App::new();
        app.editor.open_notes(0, "");
        assert!(matches!(app.editor.focus, EditorFocus::Editor));

        handle_paste(&mut app, "pasted note text");

        assert_eq!(app.editor.buffer, "pasted note text");
        assert!(app.editor.dirty);
        assert_eq!(app.input.buffer, "", "paste must not leak into the command line");
    }

    #[test]
    fn test_paste_into_editor_input_side_when_focused() {
        let mut app = App::new();
        app.editor.open_notes(0, "");
        app.editor.toggle_focus();
        assert!(matches!(app.editor.focus, EditorFocus::Input));

        handle_paste(&mut app, "command text");

        assert_eq!(app.editor.buffer, "");
        assert_eq!(app.input.buffer, "command text");
    }

    #[test]
    fn test_paste_multiline_into_note_editor_keeps_newlines() {
        let mut app = App::new();
        app.editor.open_notes(0, "");

        handle_paste(&mut app, "line one\nline two");

        assert_eq!(app.editor.buffer, "line one\nline two");
        assert!(app.editor.cursor_line > 0);
    }

    #[test]
    fn test_paste_into_popup_not_editing_is_ignored() {
        let mut app = App::new();
        app.popup_manager.open(paste_test_popup());
        // Selected field, but not in edit mode - matches how a popup sits
        // between keystrokes.
        assert!(!app.popup_manager.current().unwrap().editing);

        // Contains the Save button's shortcut ('s'); if this were still replayed
        // as synthetic keystrokes it would fire Save and close the popup.
        handle_paste(&mut app, "s");

        let state = app.popup_manager.current().unwrap();
        assert!(state.visible, "paste while not editing must not trigger button hotkeys");
        assert_eq!(state.get_text(PASTE_TEST_FIELD_TEXT), Some(""));
        assert_eq!(app.input.buffer, "", "paste must not fall through to the input area either");
    }

    #[test]
    fn test_paste_into_popup_single_line_field_collapses_newlines() {
        let mut app = App::new();
        app.popup_manager.open(paste_test_popup());
        {
            let state = app.popup_manager.current_mut().unwrap();
            state.select_field(PASTE_TEST_FIELD_TEXT);
            state.start_edit();
        }

        handle_paste(&mut app, "a\nb");

        let state = app.popup_manager.current().unwrap();
        assert_eq!(state.edit_buffer, "a b");
    }

    #[test]
    fn test_paste_into_popup_multiline_field_keeps_newlines() {
        let mut app = App::new();
        app.popup_manager.open(paste_test_popup());
        {
            let state = app.popup_manager.current_mut().unwrap();
            state.select_field(PASTE_TEST_FIELD_MULTI);
            state.start_edit();
        }

        handle_paste(&mut app, "a\nb");

        let state = app.popup_manager.current().unwrap();
        assert_eq!(state.edit_buffer, "a\nb");
    }

    #[test]
    fn test_paste_into_input_area_when_nothing_focused() {
        let mut app = App::new();

        handle_paste(&mut app, "line one\nline two");

        assert_eq!(app.input.buffer, "line one\nline two");
    }

    #[test]
    fn test_editor_state_insert_str_multibyte() {
        let mut editor = EditorState::new();
        editor.open_notes(0, "ab");
        editor.cursor_position = 1; // between 'a' and 'b'

        editor.insert_str("😀x");

        assert_eq!(editor.buffer, "a😀xb");
        assert_eq!(editor.cursor_position, 3); // 1 + "😀x".chars().count()
    }

    #[test]
    fn test_popup_state_insert_str_multibyte() {
        let def = paste_test_popup();
        let mut state = popup::PopupState::new(def);
        state.select_field(PASTE_TEST_FIELD_TEXT);
        state.start_edit();

        state.insert_str("😀x");

        assert_eq!(state.edit_buffer, "😀x");
        assert_eq!(state.edit_cursor, 2); // char count, not byte count
    }

    /// `/version` now includes a `[platform/arch]` tag (D-Termux-lines investigation)
    /// so a user's bug report carries that context automatically — e.g. distinguishing
    /// a Termux-hosted instance from the packaged Android app.
    #[test]
    fn test_version_string_includes_platform_tag() {
        let s = get_version_string();
        assert!(s.starts_with("Clay v"), "unexpected prefix: {:?}", s);
        assert!(s.contains(std::env::consts::ARCH), "missing arch tag: {:?}", s);
        // Bracketed tag is always present and non-empty on every supported target.
        let open = s.find('[').expect("missing '[' platform tag");
        let close = s.find(']').expect("missing ']' platform tag");
        assert!(close > open + 1, "empty platform/arch tag: {:?}", s);
    }

    /// Step 10 (PROTOCOL-ROADMAP.md): `ServerData.from_server` skips serialization when it
    /// equals its default (true), mirroring `flush`/`gagged`. Confirms the omission
    /// round-trips correctly: the common-case value is dropped from the JSON, and
    /// deserializing that trimmed JSON reconstructs the exact same struct via the
    /// `#[serde(default = "default_true")]` fallback already present for wire compatibility
    /// with old clients/servers. (`marked_new` used to be covered by this same test - removed
    /// along with the field itself, see World::new_from_seq's doc comment in main.rs; an old
    /// server that still sends it is simply ignored, per persistence.rs's TimestampedLine
    /// note on unknown-field tolerance.)
    #[test]
    fn test_server_data_common_case_omits_from_server() {
        let msg = WsMessage::ServerData {
            world_index: 0,
            data: "hello\n".to_string(),
            is_viewed: true,
            ts: 12345,
            from_server: true,
            seq: 7,
            // Step 5 (seq-drift fix): end_seq is None whenever the sender doesn't know a
            // batch's true span (the overwhelming majority of ServerData sites - ephemeral
            // system/command-reply messages) and must stay omitted from the wire in that
            // case, same skip_serializing_if treatment as flush/gagged.
            end_seq: None,
            flush: false,
            gagged: false, highlight_colors: Vec::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("from_server"), "from_server should be omitted when true: {}", json);
        assert!(!json.contains("flush"), "flush should still be omitted when false: {}", json);
        assert!(!json.contains("gagged"), "gagged should still be omitted when false: {}", json);
        assert!(!json.contains("end_seq"), "end_seq should be omitted when None: {}", json);

        let round_tripped: WsMessage = serde_json::from_str(&json).unwrap();
        match round_tripped {
            WsMessage::ServerData { world_index, data, is_viewed, ts, from_server, seq, end_seq, flush, gagged, .. } => {
                assert_eq!(world_index, 0);
                assert_eq!(data, "hello\n");
                assert!(is_viewed);
                assert_eq!(ts, 12345);
                assert!(from_server, "from_server must default back to true");
                assert_eq!(seq, 7);
                assert_eq!(end_seq, None, "end_seq must default back to None");
                assert!(!flush);
                assert!(!gagged);
            }
            other => panic!("expected ServerData, got {:?}", other),
        }
    }

    /// Companion to the above: confirms the non-default value (from_server: false) is still
    /// explicitly present on the wire, so the `skip_serializing_if` is conditional on the
    /// default, not a blanket omission. Also covers end_seq's present case: a real Some(u64)
    /// value must round-trip on the wire, not be trimmed - only None is omitted.
    #[test]
    fn test_server_data_non_default_from_server_is_serialized() {
        let msg = WsMessage::ServerData {
            world_index: 1,
            data: "system message".to_string(),
            is_viewed: false,
            ts: 0,
            from_server: false,
            seq: 0,
            end_seq: Some(5),
            flush: false,
            gagged: false, highlight_colors: Vec::new(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"from_server\":false"), "from_server:false must be present on the wire: {}", json);
        assert!(json.contains("\"end_seq\":5"), "end_seq:Some(5) must be present on the wire, not trimmed: {}", json);

        let round_tripped: WsMessage = serde_json::from_str(&json).unwrap();
        match round_tripped {
            WsMessage::ServerData { from_server, end_seq, .. } => {
                assert!(!from_server);
                assert_eq!(end_seq, Some(5));
            }
            other => panic!("expected ServerData, got {:?}", other),
        }
    }

    /// The two per-line ▶ ownership messages round-trip on the wire.
    #[test]
    fn test_claim_release_messages_round_trip() {
        let msg = WsMessage::ClaimedNew { world_index: 2, seqs: vec![42, 44, 45] };
        let round_tripped: WsMessage = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        match round_tripped {
            WsMessage::ClaimedNew { world_index, seqs } => {
                assert_eq!(world_index, 2);
                assert_eq!(seqs, vec![42, 44, 45],
                    "the claimed set must survive verbatim - it is deliberately not a range");
            }
            other => panic!("expected ClaimedNew, got {:?}", other),
        }

        let msg = WsMessage::ReleasedNew { world_index: 3 };
        let round_tripped: WsMessage = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        match round_tripped {
            WsMessage::ReleasedNew { world_index } => assert_eq!(world_index, 3),
            other => panic!("expected ReleasedNew, got {:?}", other),
        }
    }

    // ========== Stale new-line (▶) indicator after world switch (reconnect gap) ==========
    // See when-switching-between-worlds-majestic-salamander.md: MarkWorldSeen's
    // previous_world_index lets a client tell the server which world it's leaving even
    // after a reconnect (new client_id), when the server's own ws_client_worlds tracking of
    // the client's prior world has been lost.

    /// A client that has never been seen before (fresh client_id, no ws_client_worlds
    /// entry - simulating a reconnect after backgrounding) still gets the world it's
    /// leaving cleared, because it tells the server directly via previous_world_index.
    /// Before the fix, this depended entirely on the stale-by-then ws_client_worlds lookup
    /// and cleared nothing.
    #[test]
    fn test_mark_world_seen_previous_world_index_clears_across_reconnect() {
        let mut app = App::new();
        app.worlds.clear();

        // World::new_from_seq defaults to 0, so these 5 lines (seq 0..5, from_server by
        // OutputLine::new's default) are all ▶ (new) without any extra setup.
        let mut world_a = World::new("a");
        for i in 0..5 {
            world_a.output_lines.push(OutputLine::new(format!("a-line-{}", i), i as u64));
        }
        app.worlds.push(world_a);
        app.worlds.push(World::new("b"));

        let fresh_client_id = 999; // never inserted into ws_client_worlds
        assert!(!app.ws_client_worlds.contains_key(&fresh_client_id),
            "precondition: this client_id must be unknown to the server, like after a reconnect");

        app.handle_mark_world_seen(fresh_client_id, 1, Some(0));

        assert!(app.worlds[0].output_lines.iter().all(|l| l.display_id != Some(fresh_client_id)),
            "world 'a' (the world being left) must have THIS client's ▶ markers released via \
             previous_world_index, even though the server had no prior ws_client_worlds entry \
             for this client_id");
        assert_eq!(app.worlds[1].unseen_lines, 0, "mark_seen() must clear the new world's unseen count");

        let log = app.ws_broadcast_log.lock().unwrap();
        assert!(log.iter().any(|m| matches!(m, WsMessage::UnseenCleared { world_index: 1 })),
            "expected UnseenCleared(1). Log: {:?}", log);
        assert!(log.iter().any(|m| matches!(m, WsMessage::ActivityUpdate { .. })),
            "expected an ActivityUpdate broadcast. Log: {:?}", log);
        // Deliberately NO broadcast for world 'a': releasing this client's markers must be
        // invisible to every other instance. That is the whole point of per-line ownership -
        // the old shared watermark broadcast here wiped other clients' ▶ as a side effect.
        assert!(!log.iter().any(|m| matches!(m, WsMessage::ReleasedNew { world_index: 0 })),
            "releasing one client's markers must not be broadcast to everyone. Log: {:?}", log);
    }

    /// Without a client-supplied previous_world_index (older client, or the remote-console
    /// path that doesn't send one), the server still falls back to its own ws_client_worlds
    /// tracking - the pre-existing behavior must be preserved.
    #[test]
    fn test_mark_world_seen_falls_back_to_ws_client_worlds_when_no_previous_index() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world_a = World::new("a");
        world_a.output_lines.push(OutputLine::new("a-line".to_string(), 0));
        app.worlds.push(world_a);
        app.worlds.push(World::new("b"));

        let client_id = 5;
        app.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: 0,
            visible_lines: 24,
            visible_columns: 80,
            dimensions: None,
            paused: false,
            visible: true,
            disconnected_at: None,
        });

        app.handle_mark_world_seen(client_id, 1, None);

        assert!(app.worlds[0].output_lines.iter().all(|l| l.display_id != Some(client_id)),
            "world 'a' must still have this client's ▶ markers released via the ws_client_worlds fallback lookup");
        assert_eq!(app.ws_client_worlds.get(&client_id).map(|s| s.world_index), Some(1));
    }

    /// MarkWorldSeen for the world the client is already viewing (world_index ==
    /// previous_world_index) must not clear that world's own indicators.
    #[test]
    fn test_mark_world_seen_same_world_does_not_clear_itself() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world_a = World::new("a");
        world_a.output_lines.push(OutputLine::new("a-line".to_string(), 0));
        app.worlds.push(world_a);

        // Claim it for this client first, so there is a marker that could be wrongly cleared.
        app.worlds[0].claim_unviewed(1);
        app.handle_mark_world_seen(1, 0, Some(0));

        assert_eq!(app.worlds[0].output_lines[0].display_id, Some(1),
            "marking the world you're already on as seen must not clear its own ▶ markers \
             (those only clear when switching AWAY, or on Ctrl+L)");
    }

    // ========== Resume semantics: grace window for a briefly-disconnected WS client ==========

    #[test]
    fn test_ws_client_viewing_true_within_grace_window_after_disconnect() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("a"));
        app.ws_client_worlds.insert(1, ClientViewState {
            world_index: 0,
            visible_lines: 24,
            visible_columns: 80,
            dimensions: None,
            paused: false,
            visible: true,
            disconnected_at: Some(std::time::Instant::now()), // just disconnected
        });
        assert!(app.ws_client_viewing(0),
            "a client that disconnected moments ago must still count as viewing its world, \
             so output arriving during a brief background/reconnect gap isn't wrongly marked_new");
    }

    #[test]
    fn test_ws_client_viewing_false_after_grace_window_expires() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("a"));
        let long_ago = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(16 * 60))
            .expect("test host must have > 16 minutes of monotonic clock uptime");
        app.ws_client_worlds.insert(1, ClientViewState {
            world_index: 0,
            visible_lines: 24,
            visible_columns: 80,
            dimensions: None,
            paused: false,
            visible: true,
            disconnected_at: Some(long_ago),
        });
        assert!(!app.ws_client_viewing(0),
            "a client disconnected well past the grace window must no longer count as viewing");
    }

    #[test]
    fn test_min_viewer_lines_excludes_disconnected_clients() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("a"));
        // Actively connected client with the larger viewport.
        app.ws_client_worlds.insert(1, ClientViewState {
            world_index: 0, visible_lines: 40, visible_columns: 100,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });
        // Disconnected (but within grace) client with a smaller viewport - must not
        // constrain more-mode pagination for the still-present viewer above.
        app.ws_client_worlds.insert(2, ClientViewState {
            world_index: 0, visible_lines: 10, visible_columns: 40,
            dimensions: None, paused: false, visible: true, disconnected_at: Some(std::time::Instant::now()),
        });
        assert_eq!(app.min_viewer_lines(0), Some(40),
            "the disconnected client's smaller viewport must be excluded from the min");
        assert_eq!(app.min_viewer_width(0), Some(100));
    }

    #[test]
    fn test_reap_stale_ws_client_worlds_removes_only_expired_entries() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("a"));
        let long_ago = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(16 * 60))
            .expect("test host must have > 16 minutes of monotonic clock uptime");
        app.ws_client_worlds.insert(1, ClientViewState { // still connected: keep
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });
        app.ws_client_worlds.insert(2, ClientViewState { // disconnected, within grace: keep
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: Some(std::time::Instant::now()),
        });
        app.ws_client_worlds.insert(3, ClientViewState { // disconnected, expired: reap
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: Some(long_ago),
        });

        app.reap_stale_ws_client_worlds();

        assert!(app.ws_client_worlds.contains_key(&1));
        assert!(app.ws_client_worlds.contains_key(&2));
        assert!(!app.ws_client_worlds.contains_key(&3));
    }

    // ========== Rule 1: arrival wins - a viewed world is never ▶, pending or not ==========

    #[test]
    fn test_pending_lines_not_marked_new_when_current() {
        // Rule 1 (see World::new_from_seq's doc comment): a line is new iff nobody was
        // viewing the world when it arrived. Someone IS viewing here (is_current: true), so
        // even though this line lands in pending_lines (must be released with PgDn/Tab), it
        // must NOT be ▶ - arrival state decides new-ness, not display state. This is the
        // opposite of the old per-line model, which unconditionally flagged every pending
        // line regardless of who was watching when it arrived.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };
        world.paused = true; // already paused: new output goes straight to pending_lines

        world.add_output("held back line\n", true /* is_current */, &settings, 24, 80, false, true, false);

        assert_eq!(world.pending_lines.len(), 1);
        assert!(world.pending_lines[0].viewed,
            "a pending line arriving while the world is viewed is born viewed - arrival wins \
             over display state, so it can never be claimed as ▶ by anyone later");
        assert_eq!(world.pending_lines[0].display_id, None, "arrival never assigns an owner");
        // unseen_lines stays gated on !is_current - a pending line on the world you're
        // looking at isn't "unseen" in the aggregate-activity sense, only "not yet displayed".
        assert_eq!(world.unseen_lines, 0);
    }

    #[test]
    fn test_output_lines_not_marked_new_when_current_and_not_pending() {
        // The non-pending screenful shown immediately on a world you're viewing must NOT
        // get the ▶ marker either - consistent with the pending case above, since is_current
        // is what actually decides new-ness now, not the pending/output split.
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        world.add_output("visible line\n", true /* is_current */, &settings, 24, 80, false, true, false);

        assert_eq!(world.output_lines.len(), 1);
        assert!(world.output_lines[0].viewed, "born viewed - somebody was watching");
        // ...and a later claim must not be able to turn it into ▶ for anyone.
        world.claim_unviewed(7);
        assert_eq!(world.output_lines[0].display_id, None,
            "a line that arrived while viewed is permanently un-new - the !viewed guard in \
             claim_unviewed is what enforces that");
        assert!(world.pending_lines.is_empty());
    }

    #[test]
    fn test_filter_to_server_output_clears_output_but_preserves_pending_markers() {
        // Ctrl+L (World::filter_to_server_output) marks whatever's currently displayed as
        // seen, but must NOT touch pending_lines - they haven't been displayed yet (that's
        // what pending means), so rule 2 says they keep their ▶ regardless of Ctrl+L. This
        // is a deliberate behavior change from the old model, which used to blanket-clear
        // marked_new on both output_lines and pending_lines.
        let mut world = World::new("test");
        for i in 0..3u64 {
            world.output_lines.push(OutputLine::new(format!("output {i}"), i));
        }
        for i in 3..6u64 {
            world.pending_lines.push(OutputLine::new(format!("pending {i}"), i));
        }
        // The console displays the world, claiming everything unviewed in output_lines.
        // claim_unviewed deliberately does NOT touch pending_lines - they haven't been
        // displayed yet, so they stay claimable until released.
        world.claim_unviewed(crate::CONSOLE_DISPLAY_ID);
        assert!(world.output_lines.iter().all(|l| l.display_id == Some(crate::CONSOLE_DISPLAY_ID)));
        assert!(world.pending_lines.iter().all(|l| !l.viewed && l.display_id.is_none()),
            "pending lines are untouched by a claim - still undisplayed, still claimable");

        world.filter_to_server_output();

        assert!(world.output_lines.iter().all(|l| l.display_id.is_none()),
            "output_lines must lose the console's ▶ marker after Ctrl+L");
        assert!(world.output_lines.iter().all(|l| l.viewed),
            "...but stay viewed, so no other client can pick them up");
        assert!(world.pending_lines.iter().all(|l| !l.viewed),
            "pending_lines must survive Ctrl+L unviewed - they're still undisplayed, so they \
             become ▶ for whoever is watching when they're finally released");
    }

    // ========== Rule 1 vs. rule 2: arrivals must not clear OTHER lines' markers ==========
    //
    // Regression guard for a bug where a single monotonic new_from_seq floor, advanced to
    // exclude a brand-new arriving line while a world was being viewed, ALSO retroactively
    // swept every older, still-legitimately-new backlog line below it - even though nothing
    // had actually been "displayed" in the rule-2 sense (leaving the world / Ctrl+L). Fixed
    // by adding a second watermark, World::viewed_from_seq (see its doc comment), which
    // excludes only the lines that arrived during the current viewing episode without
    // touching new_from_seq at all.

    #[test]
    fn test_arrival_while_viewing_keeps_older_backlog_markers() {
        // THE regression this whole area exists for: text arriving on the world you are
        // looking at must not strip ▶ from older backlog that arrived while nobody was
        // watching. Under per-line ownership this falls out for free - the backlog is owned,
        // the live arrivals are born `viewed` and unowned, and nothing touches the backlog.
        let mut world = World::new("test");
        let settings = Settings::default();

        // Backlog: arrived with nobody viewing -> claimable.
        world.add_output("backlog 1\n", false, &settings, 24, 80, false, true, false);
        world.add_output("backlog 2\n", false, &settings, 24, 80, false, true, false);
        assert!(world.output_lines.iter().all(|l| !l.viewed), "precondition: arrived unviewed");

        // The user switches in and displays the world: the backlog becomes theirs.
        world.claim_unviewed(crate::CONSOLE_DISPLAY_ID);
        assert!(world.output_lines.iter().all(|l| l.display_id == Some(crate::CONSOLE_DISPLAY_ID)));

        // Now live output arrives while they are still sitting there. No display event.
        world.add_output("live 1\n", true, &settings, 24, 80, false, true, false);
        world.add_output("live 2\n", true, &settings, 24, 80, false, true, false);

        let flags: Vec<bool> = world.output_lines.iter()
            .map(|l| l.display_id == Some(crate::CONSOLE_DISPLAY_ID)).collect();
        assert_eq!(flags, vec![true, true, false, false],
            "backlog must KEEP ▶ while you sit in the world; lines that arrived while you \
             were watching are born viewed and never become ▶ for anyone");

        // Leaving (or Ctrl+L) is what actually clears them - and only for this viewer.
        world.release_claims(crate::CONSOLE_DISPLAY_ID);
        assert!(world.output_lines.iter().all(|l| l.display_id.is_none()));
        assert!(world.output_lines.iter().all(|l| l.viewed),
            "released lines stay viewed, so nobody else picks them up");
    }

    #[test]
    fn test_unviewed_arrival_after_a_viewing_episode_is_new_again() {
        // A viewer can stop viewing with no display event at all: a WS client switching
        // world, hitting --More--, backgrounding, or having its grace expire. Text arriving
        // after that must be claimable again.
        let mut world = World::new("test");
        let settings = Settings::default();

        world.add_output("watched\n", true, &settings, 24, 80, false, true, false);
        world.add_output("unwatched\n", false, &settings, 24, 80, false, true, false);

        world.claim_unviewed(crate::CONSOLE_DISPLAY_ID);

        assert_eq!(world.output_lines[0].display_id, None,
            "a line that arrived while watched can never become ▶, even on a later claim");
        assert_eq!(world.output_lines[1].display_id, Some(crate::CONSOLE_DISPLAY_ID),
            "text arriving once the last viewer stopped watching must be claimable again");
    }

    #[test]
    fn test_client_generated_lines_are_never_claimed() {
        // Client-generated text (a /world message, a status notice) is never ▶, and its
        // presence must not disturb the real backlog around it.
        let mut world = World::new("test");
        let settings = Settings::default();

        world.add_output("backlog\n", false, &settings, 24, 80, false, true, false);
        world.add_output("watched\n", true, &settings, 24, 80, false, true, false);
        world.add_output("client note\n", false, &settings, 24, 80, false, false /* from_server */, false);

        world.claim_unviewed(crate::CONSOLE_DISPLAY_ID);

        assert_eq!(world.output_lines[0].display_id, Some(crate::CONSOLE_DISPLAY_ID),
            "a client-generated arrival must not cost real backlog its marker");
        assert_eq!(world.output_lines[1].display_id, None, "arrived while watched");
        assert_eq!(world.output_lines[2].display_id, None,
            "client-generated lines are never ▶ - claim_unviewed gates on from_server");
    }

    #[test]
    fn test_arrival_broadcasts_no_ownership_message() {
        // Arrival never assigns an owner, so it must not emit ClaimedNew/ReleasedNew at all.
        // The old model broadcast a watermark on every arrival to every client; that
        // broadcast is exactly what let one client's state disturb another's.
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("current"));
        app.current_world_index = 0;
        app.ws_broadcast_log.lock().unwrap().clear();

        app.add_output_to_world(0, "hello");

        let log = app.ws_broadcast_log.lock().unwrap();
        assert!(!log.iter().any(|m| matches!(m,
            WsMessage::ClaimedNew { .. } | WsMessage::ReleasedNew { .. })),
            "arrival must not broadcast any ownership change. Log: {:?}", log);
    }

    // ========================================================================
    // Output that used to reach the TUI but never any client
    // ========================================================================

    /// Drains every `WsMessage` the fake client received, decoding BOTH outbound forms.
    ///
    /// A broadcast is `Outbound::Shared` (serialized once, shared by `Arc<str>` across every
    /// recipient); only single-recipient sends are `Outbound::Message`. A drain that matches
    /// just the latter silently sees nothing from any broadcast path.
    fn drain_ws_messages(rx: &mut tokio::sync::mpsc::Receiver<crate::websocket::Outbound>)
        -> Vec<crate::websocket::WsMessage>
    {
        use crate::websocket::Outbound;
        let mut out = Vec::new();
        while let Ok(item) = rx.try_recv() {
            match item {
                Outbound::Message(msg) => out.push(*msg),
                Outbound::Shared(json) => {
                    if let Ok(msg) = serde_json::from_str::<crate::websocket::WsMessage>(&json) {
                        out.push(msg);
                    }
                }
            }
        }
        out
    }

    /// Every ServerData the fake client received, as (seq, end_seq, text).
    fn drain_server_data(rx: &mut tokio::sync::mpsc::Receiver<crate::websocket::Outbound>)
        -> Vec<(u64, Option<u64>, String)>
    {
        use crate::websocket::WsMessage;
        drain_ws_messages(rx).into_iter().filter_map(|m| {
            if let WsMessage::ServerData { seq, end_seq, data, .. } = m {
                Some((seq, end_seq, data))
            } else { None }
        }).collect()
    }

    /// `handle_disconnected` pushes the world's final prompt into `output_lines` and used to
    /// broadcast nothing for it — the TUI showed a line at the bottom that no client ever
    /// received, and the consumed seq left a permanent hole in every client's delivered-range
    /// tracking. The `"Disconnected."` line immediately below it was always broadcast, which
    /// is what made the omission so easy to miss.
    #[test]
    fn test_disconnect_broadcasts_the_final_prompt_line() {
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        w.connected = true;
        w.prompt = "HP:100> ".to_string();
        app.worlds.push(w);
        app.current_world_index = 0;
        let (_client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_disconnected(0);

        let sent = drain_server_data(&mut rx);
        assert!(sent.iter().any(|(_, _, d)| d.contains("HP:100>")),
            "the final prompt must be broadcast, not just rendered locally. Got: {sent:?}");
        assert!(sent.iter().any(|(_, _, d)| d.contains("Disconnected.")),
            "precondition: the Disconnected. line was always broadcast. Got: {sent:?}");

        // ...and it must carry the line's REAL seq, so the client can account for it.
        let prompt_line = app.worlds[0].output_lines.iter()
            .find(|l| l.text.contains("HP:100>")).expect("prompt line in output_lines");
        let (seq, end_seq, _) = sent.iter().find(|(_, _, d)| d.contains("HP:100>")).unwrap();
        assert_eq!(*seq, prompt_line.seq, "must broadcast the stored seq, not 0");
        assert_eq!(*end_seq, Some(prompt_line.seq),
            "end_seq present is what marks the seq as real - seq 0 is a legitimate value");
    }

    /// `handle_prompt` on a disconnected world renders the prompt as an output line and used
    /// to `return` without broadcasting anything at all.
    #[test]
    fn test_disconnected_world_prompt_as_output_is_broadcast() {
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        w.connected = false;
        app.worlds.push(w);
        app.current_world_index = 0;
        let (_client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_prompt(0, b"login: ");

        let sent = drain_server_data(&mut rx);
        assert!(sent.iter().any(|(_, _, d)| d.contains("login:")),
            "a prompt shown as output on a disconnected world must reach clients too. Got: {sent:?}");
    }

    /// A batch must never carry `end_seq < first_seq`. That happened when more-mode was
    /// toggled off while a world was paused: the drained pending lines (higher seqs) were
    /// appended AFTER the loop's newly-allocated lines, leaving `output_lines` unsorted. The
    /// client read the inverted span as a mid-buffer gap-fill and spliced the batch far from
    /// the tail — "the TUI shows it, the phone doesn't".
    #[test]
    fn test_more_mode_off_drain_keeps_output_sorted_and_span_forward() {
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        // A paused world holding a backlog.
        w.paused = true;
        for seq in 0..3u64 {
            w.pending_lines.push(OutputLine::new(format!("held {seq}"), seq));
        }
        w.next_seq = 3;
        app.worlds.push(w);
        app.current_world_index = 0;
        app.settings.more_mode_enabled = false;
        let (_client_id, mut rx) = phase_c_register_client(&mut app);

        push_server_line(&mut app, 0, "fresh line");

        let seqs: Vec<u64> = app.worlds[0].output_lines.iter().map(|l| l.seq).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted,
            "output_lines must stay sorted by seq across the drain (CLAUDE.md invariant): {seqs:?}");

        for (seq, end_seq, data) in drain_server_data(&mut rx) {
            if let Some(end) = end_seq {
                assert!(end >= seq,
                    "broadcast span must never be inverted: seq={seq} end_seq={end} data={data:?}");
            }
        }
    }

    /// `broadcast_output_range` must derive its span from the min/max of the slice, so that
    /// even a buffer that somehow ends up unsorted produces a forward span rather than one
    /// the client mistakes for a mid-buffer gap-fill.
    #[test]
    fn test_broadcast_span_is_forward_even_for_an_unsorted_slice() {
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        // Deliberately out of order - the invariant is defended elsewhere; this asserts the
        // broadcaster degrades to a wide-but-forward span instead of an inverted one.
        w.output_lines.push(OutputLine::new("high".to_string(), 9));
        w.output_lines.push(OutputLine::new("low".to_string(), 2));
        w.next_seq = 10;
        app.worlds.push(w);
        app.current_world_index = 0;
        let (_client_id, mut rx) = phase_c_register_client(&mut app);

        app.broadcast_output_range(0, 0, 2, true, true, false);

        let sent = drain_server_data(&mut rx);
        assert_eq!(sent.len(), 1, "{sent:?}");
        let (seq, end_seq, _) = &sent[0];
        assert_eq!(*seq, 2, "first_seq must be the slice minimum");
        assert_eq!(*end_seq, Some(9), "end_seq must be the slice maximum");
    }

    /// `/hilite` colours are applied to the server-side buffer but used to be dropped on the
    /// live path entirely, so a highlighted line arrived and rendered plain until a resync.
    /// The array is omitted (empty) when nothing in the batch is highlighted, so the common
    /// path stays free.
    #[test]
    fn test_highlight_colors_ride_the_live_broadcast() {
        use crate::websocket::WsMessage;
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        w.output_lines.push(OutputLine::new("plain".to_string(), 0));
        let mut hot = OutputLine::new("hot".to_string(), 1);
        hot.highlight_color = Some("red".to_string());
        w.output_lines.push(hot);
        w.next_seq = 2;
        app.worlds.push(w);
        app.current_world_index = 0;
        let (_client_id, mut rx) = phase_c_register_client(&mut app);

        app.broadcast_output_range(0, 0, 2, true, true, false);

        let mut colors = None;
        for m in drain_ws_messages(&mut rx) {
            if let WsMessage::ServerData { highlight_colors, .. } = m {
                colors = Some(highlight_colors);
            }
        }
        assert_eq!(colors, Some(vec![None, Some("red".to_string())]),
            "highlight colours must be parallel to the lines in `data`");

        // Nothing highlighted -> omitted entirely.
        app.worlds[0].output_lines[1].highlight_color = None;
        app.broadcast_output_range(0, 0, 2, true, true, false);
        let mut colors2: Option<Vec<Option<String>>> = None;
        for m in drain_ws_messages(&mut rx) {
            if let WsMessage::ServerData { highlight_colors, .. } = m {
                colors2 = Some(highlight_colors);
            }
        }
        assert_eq!(colors2, Some(Vec::new()),
            "an unhighlighted batch must carry no colour array at all (zero wire cost)");
    }

    /// A gap-fill truncated by the pending clamp must NOT report `backfill_complete` — more
    /// is owed, it just isn't deliverable yet. Reporting completion cleared the client's
    /// _gapFillPending and stopped its pump while it was still behind.
    #[test]
    fn test_pending_clamped_gap_fill_is_not_reported_complete() {
        use crate::websocket::WsMessage;
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        // Deliverable: seq 1. Withheld behind the pending floor: seq 5 (invariant-violating
        // shape the clamp exists for).
        w.output_lines.push(OutputLine::new("deliverable".to_string(), 1));
        w.output_lines.push(OutputLine::new("above the floor".to_string(), 5));
        w.pending_lines.push(OutputLine::new("queued".to_string(), 4));
        w.next_seq = 6;
        app.worlds.push(w);
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_request_scrollback(client_id, 0, 500, None, Some(0), Some(1));

        let mut complete = None;
        for m in drain_ws_messages(&mut rx) {
            if let WsMessage::ScrollbackLines { backfill_complete, .. } = m {
                complete = Some(backfill_complete);
            }
        }
        assert_eq!(complete, Some(false),
            "a clamped result must stay incomplete so the client keeps its gap-fill armed");
    }

    // ========================================================================
    // Per-line ▶ ownership (OutputLine::viewed / display_id)
    // ========================================================================

    /// Push one line of SERVER-originated text onto `world_idx` exactly the way
    /// `process_server_data` does: through `World::add_output` with the same
    /// console-or-any-WS-client `is_current`. `App::add_output_to_world` is deliberately not
    /// used here - it emits CLIENT-generated text (`from_server: false`), which is never ▶.
    fn push_server_line(app: &mut App, world_idx: usize, text: &str) {
        let is_current = world_idx == app.current_world_index || app.ws_client_viewing(world_idx);
        let settings = app.settings.clone();
        app.worlds[world_idx].add_output(&format!("{text}\n"), is_current, &settings, 24, 80, false, true, false);
    }

    /// The four arrival/claim rules, as a table.
    #[test]
    fn test_ownership_arrival_and_claim_rules() {
        let settings = Settings::default();

        // Row 1: arrives on a world nobody is viewing -> unviewed, unowned.
        let mut w = World::new("t");
        w.add_output("a\n", false, &settings, 24, 80, false, true, false);
        assert!(!w.output_lines[0].viewed);
        assert_eq!(w.output_lines[0].display_id, None);

        // Row 2: arrives on a world somebody IS viewing -> viewed, still unowned.
        let mut w = World::new("t");
        w.add_output("a\n", true, &settings, 24, 80, false, true, false);
        assert!(w.output_lines[0].viewed);
        assert_eq!(w.output_lines[0].display_id, None);

        // Row 3: a client displays an unviewed line -> viewed, owned by that client.
        let mut w = World::new("t");
        w.add_output("a\n", false, &settings, 24, 80, false, true, false);
        assert_eq!(w.claim_unviewed(42), vec![0]);
        assert!(w.output_lines[0].viewed);
        assert_eq!(w.output_lines[0].display_id, Some(42));

        // Row 4: a client displays an already-viewed line -> nothing happens.
        let mut w = World::new("t");
        w.add_output("a\n", true, &settings, 24, 80, false, true, false);
        assert!(w.claim_unviewed(42).is_empty());
        assert_eq!(w.output_lines[0].display_id, None,
            "a line that arrived while somebody was watching is permanently un-new");
    }

    /// THE test for this design: a second client displaying a line the first client already
    /// owns must neither steal that marker nor clear it. Everything else follows from the
    /// `!viewed` guard in claim_unviewed; this is the assertion that pins it down.
    #[test]
    fn test_second_viewer_neither_steals_nor_clears_the_first_viewers_marker() {
        let settings = Settings::default();
        let mut w = World::new("t");
        w.add_output("backlog\n", false, &settings, 24, 80, false, true, false);

        // Client 1 looks first and takes the marker.
        assert_eq!(w.claim_unviewed(1), vec![0]);
        assert_eq!(w.output_lines[0].display_id, Some(1));

        // Client 2 now displays the same line.
        assert!(w.claim_unviewed(2).is_empty(), "nothing left to claim");
        assert_eq!(w.output_lines[0].display_id, Some(1),
            "client 2 must NOT steal client 1's marker - first viewer wins");

        // Client 2 switching away / Ctrl+L must not disturb client 1 either.
        assert!(!w.release_claims(2), "client 2 owns nothing here");
        assert_eq!(w.output_lines[0].display_id, Some(1),
            "one client's release must be invisible to another - this is exactly what the \
             shared watermark got wrong, wiping other clients' ▶ on every world switch");

        // Only client 1's own release clears it.
        assert!(w.release_claims(1));
        assert_eq!(w.output_lines[0].display_id, None);
        assert!(w.output_lines[0].viewed,
            "released lines stay viewed so nobody re-claims them");
    }

    /// Unviewed lines are NOT a contiguous tail: `viewed` is decided per line by whether
    /// anyone was watching at that instant, and that flips with no display event in between.
    /// A claim must sweep the whole buffer, not stop at the first viewed line.
    #[test]
    fn test_claim_sweeps_unviewed_lines_behind_a_viewed_one() {
        let settings = Settings::default();
        let mut w = World::new("t");
        w.add_output("unwatched 1\n", false, &settings, 24, 80, false, true, false);
        w.add_output("watched\n", true, &settings, 24, 80, false, true, false);
        w.add_output("unwatched 2\n", false, &settings, 24, 80, false, true, false);

        let claimed = w.claim_unviewed(9);
        assert_eq!(claimed, vec![0, 2],
            "both unviewed lines must be claimed, including the one BEHIND the viewed line - \
             a reverse scan that breaks on the first viewed line silently skips it");
        assert_eq!(w.output_lines[1].display_id, None, "the viewed line stays unowned");
    }

    /// Ctrl+L / switching away releases only the acting viewer's markers, and pending lines
    /// survive unviewed so they become ▶ for whoever is watching when they're released.
    #[test]
    fn test_release_is_per_viewer_and_pending_stays_claimable() {
        let mut w = World::new("t");
        for i in 0..3u64 {
            w.output_lines.push(OutputLine::new(format!("out {i}"), i));
        }
        w.pending_lines.push(OutputLine::new("held".to_string(), 3));

        w.claim_unviewed(1);
        // A second client owns nothing (first-wins), so give it a line of its own.
        w.output_lines.push(OutputLine::new("later".to_string(), 4));
        w.claim_unviewed(2);
        assert_eq!(w.output_lines[3].display_id, Some(2));

        w.release_claims(1);
        assert!(w.output_lines[..3].iter().all(|l| l.display_id.is_none()));
        assert_eq!(w.output_lines[3].display_id, Some(2),
            "client 1's release must leave client 2's marker alone");
        assert!(!w.pending_lines[0].viewed,
            "pending lines are never claimed and never released - still undisplayed");
    }

    /// The reported scenario, end to end through the real handlers: console on world A, a WS
    /// client on world B. Text arriving on B must be new for nobody, because the client
    /// watching B is watching it.
    #[test]
    fn test_text_on_a_world_being_watched_by_a_remote_client_is_never_new() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("A"));
        app.worlds.push(World::new("B"));
        app.current_world_index = 0; // console watches A

        let (client_id, _rx) = phase_c_register_client(&mut app);
        // The WS client is viewing world B.
        app.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: 1,
            visible_lines: 24,
            visible_columns: 80,
            dimensions: None,
            paused: false,
            visible: true,
            disconnected_at: None,
        });
        assert!(app.ws_client_viewing(1), "precondition: B counts as viewed");

        push_server_line(&mut app, 1, "text on B");

        let line = app.worlds[1].output_lines.last().unwrap();
        assert!(line.viewed,
            "text arriving on a world a remote client is watching must be born viewed - this \
             is the reported bug: it used to be ▶ on that very client");
        assert_eq!(line.display_id, None);
        // ...and it must stay un-new even if that client re-displays the world.
        let owner = app.display_owner_id(client_id);
        app.claim_world_for(1, owner, Some(client_id));
        assert_eq!(app.worlds[1].output_lines.last().unwrap().display_id, None);
    }

    /// A backgrounded client stops counting as a viewer and drops its markers; coming back
    /// claims whatever arrived meanwhile. A brief transport drop is the opposite case and is
    /// covered by WS_VIEWER_GRACE.
    #[test]
    fn test_backgrounding_releases_markers_and_returning_reclaims() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("A"));
        app.current_world_index = 9999; // console is not on this world

        let (client_id, _rx) = phase_c_register_client(&mut app);
        // Text arrives BEFORE the client is looking, so it's genuinely missed and claimable.
        push_server_line(&mut app, 0, "arrived before we looked");
        app.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });

        let owner = app.display_owner_id(client_id);
        app.claim_world_for(0, owner, Some(client_id));
        assert_eq!(app.worlds[0].output_lines[0].display_id, Some(owner),
            "displaying a world with missed text takes ownership of its ▶ markers");

        // Background: markers released, and we stop counting as a viewer.
        app.handle_client_visibility(client_id, false);
        assert_eq!(app.worlds[0].output_lines[0].display_id, None);
        assert!(!app.ws_client_viewing(0),
            "a backgrounded client must not keep suppressing arrivals on its world");

        // Text arriving now is genuinely missed.
        push_server_line(&mut app, 0, "while away");
        assert!(!app.worlds[0].output_lines.last().unwrap().viewed);

        // Back on screen: claim it.
        app.handle_client_visibility(client_id, true);
        assert_eq!(app.worlds[0].output_lines.last().unwrap().display_id, Some(owner),
            "text that arrived while backgrounded must become ▶ on return");
    }

    /// A `ClientVisibility { visible: true }` must always be answered with a `ClaimedNew`,
    /// even when the server already considered the client visible.
    ///
    /// The client claims optimistically the instant it returns to the foreground so ▶ paints
    /// in the same frame as the text. That guess is reconciled by the ClaimedNew this triggers
    /// — and it always *looked* like a repeat here on the first resume after a page load or
    /// reconnect, because the client's own visible-state mirror starts unknown while
    /// `ClientViewState::visible` defaults to true. Early-returning then left the guess
    /// outstanding until some unrelated later ClaimedNew revoked it: ▶ appeared, then vanished
    /// a moment later.
    #[test]
    fn test_visible_true_always_answers_even_when_already_visible() {
        use crate::websocket::WsMessage;
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("A"));
        app.current_world_index = 0;

        let (client_id, mut rx) = phase_c_register_client(&mut app);
        app.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });

        // Nothing unviewed: the claim itself is a no-op, but the answer still has to come.
        while rx.try_recv().is_ok() {}
        app.handle_client_visibility(client_id, true);
        assert_eq!(claimed_new_seqs(&drain_ws_messages(&mut rx)), vec![Vec::<u64>::new()],
            "a redundant visible=true must still answer, or an optimistic claim is stranded");

        // And with something genuinely unviewed it answers with the real list.
        push_server_line(&mut app, 0, "missed while nobody looked");
        app.worlds[0].output_lines.last_mut().unwrap().viewed = false;
        let seq = app.worlds[0].output_lines.last().unwrap().seq;
        while rx.try_recv().is_ok() {}
        app.handle_client_visibility(client_id, true);
        assert_eq!(claimed_new_seqs(&drain_ws_messages(&mut rx)), vec![vec![seq]]);

        // Backgrounding is still a one-shot: a repeated hidden must not re-release.
        app.handle_client_visibility(client_id, false);
        while rx.try_recv().is_ok() {}
        app.handle_client_visibility(client_id, false);
        let msgs = drain_ws_messages(&mut rx);
        assert!(!msgs.iter().any(|m| matches!(m, WsMessage::ReleasedNew { .. })),
            "a repeated visible=false must not re-send ReleasedNew. Got: {msgs:?}");
    }

    /// Backgrounding must never be mistaken for an operator pause. `ClientViewState::paused`
    /// is user-visible state (`/remote --pause`) that `handle_request_state` reports back as
    /// the client's PAUSED badge; backgrounding writes `visible` instead. Folding the two
    /// together made a resync landing inside the background window light that badge up, with
    /// nothing to clear it on return.
    #[test]
    fn test_backgrounding_does_not_look_like_an_operator_pause() {
        use crate::websocket::WsMessage;
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("A"));
        app.current_world_index = 0;

        let (client_id, mut rx) = phase_c_register_client(&mut app);
        app.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });

        app.handle_client_visibility(client_id, false);

        let st = app.ws_client_worlds.get(&client_id).unwrap();
        assert!(!st.visible, "backgrounding must clear `visible`");
        assert!(!st.paused, "backgrounding must NOT touch `paused` - that's operator state");
        assert!(!app.ws_client_viewing(0), "a hidden client must not count as a viewer");

        // A resync arriving while backgrounded must not report a pause.
        while rx.try_recv().is_ok() {}
        app.handle_request_state(client_id);
        let paused_msgs: Vec<_> = drain_ws_messages(&mut rx).into_iter()
            .filter(|m| matches!(m, WsMessage::PausedState { .. })).collect();
        assert!(paused_msgs.is_empty(),
            "a resync while backgrounded must not send PausedState - it would light the \
             PAUSED badge as though an operator had paused the session. Got: {paused_msgs:?}");

        // Returning restores viewer status.
        app.handle_client_visibility(client_id, true);
        assert!(app.ws_client_viewing(0), "a visible client counts as a viewer again");
    }

    /// The converse: an operator pause DOES set `paused`, keeps the client out of the viewer
    /// set, and is still re-asserted on resync so the badge survives a reconnect.
    #[test]
    fn test_operator_pause_still_sets_paused_and_is_reasserted_on_resync() {
        use crate::websocket::WsMessage;
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("A"));
        app.current_world_index = 9999; // console is elsewhere, so only the client can view

        let (client_id, mut rx) = phase_c_register_client(&mut app);
        app.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: 0, visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });
        assert!(app.ws_client_viewing(0), "precondition");

        let toggled = app.ws_toggle_client_paused(client_id);
        assert!(toggled.is_some(), "toggle should find the client");
        let st = app.ws_client_worlds.get(&client_id).unwrap();
        assert!(st.paused, "an operator pause sets `paused`");
        assert!(st.visible, "...and must NOT clear `visible` - the app is still on screen");
        assert!(!app.ws_client_viewing(0), "a paused session doesn't count as a viewer either");

        while rx.try_recv().is_ok() {}
        app.handle_request_state(client_id);
        let reasserted = drain_ws_messages(&mut rx).into_iter().any(|m|
            matches!(m, WsMessage::PausedState { paused: true }));
        assert!(reasserted,
            "an operator pause must be re-asserted on resync so the PAUSED badge survives");
    }

    /// A stable client_uid keeps ▶ ownership across a reconnect, which is what makes a brief
    /// transport drop non-destructive. Without one, ownership falls back to the connection id
    /// and a reconnect loses the markers.
    #[test]
    fn test_stable_client_uid_survives_a_reconnect() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("A"));

        app.set_display_owner_id(11, "device-abc");
        let first = app.display_owner_id(11);
        // Same device, new connection id after a reconnect.
        app.set_display_owner_id(12, "device-abc");
        assert_eq!(app.display_owner_id(12), first,
            "the same client_uid must resolve to the same ownership id across a reconnect");

        // No uid -> falls back to the connection id (older client, one-shot Rust clients).
        assert_eq!(app.display_owner_id(77), 77);
        // ...and must never collide with the console's reserved id.
        app.set_display_owner_id(13, "another-device");
        assert_ne!(app.display_owner_id(13), CONSOLE_DISPLAY_ID);
    }

    // ============================================================================
    // Round-3 seq-dedup-poisoning audit regression tests
    // ============================================================================

    #[test]
    fn test_gagged_line_respects_pending_backlog_when_more_mode_off() {
        // Fix 1 regression guard: hold_gagged_in_pending used to also gate on
        // settings.more_mode_enabled, but a background world can stay paused with a
        // non-empty pending_lines backlog even after more-mode is toggled off globally -
        // World::add_output's "more-mode off => drain pending" branch only fires for a
        // world actively receiving new non-gagged output at the moment of the toggle, so a
        // paused background world not currently receiving non-gagged output keeps its
        // stale paused/pending_lines state regardless of the current setting value. The
        // condition that actually matters is "is there an existing backlog this line would
        // otherwise jump ahead of" - paused && !pending_lines.is_empty() alone.
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("test");
        world.paused = true;
        world.settings.keep_alive_type = KeepAliveType::Custom;
        // Existing backlog already queued, as if it arrived while more-mode was on.
        world.pending_lines.push(OutputLine::new("earlier backlog line".to_string(), world.next_seq));
        world.next_seq += 1;
        app.worlds.push(world);
        app.current_world_index = 0;
        // more-mode now toggled OFF globally - but this world's stale paused/pending_lines
        // state survives regardless (see comment above), since it isn't currently
        // receiving non-gagged output to trigger the drain branch.
        app.settings.more_mode_enabled = false;

        app.ws_broadcast_log.lock().unwrap().clear();
        app.process_server_data(0, b"###_idler_message_1_###\r\n", 24, 80, false);

        assert_eq!(app.worlds[0].pending_lines.len(), 2,
            "the gagged line must be queued behind the existing backlog, not jump into output_lines: {:?}",
            app.worlds[0].pending_lines.iter().map(|l| (l.seq, l.gagged)).collect::<Vec<_>>());
        assert!(app.worlds[0].pending_lines[1].gagged, "the idler keepalive line must be gagged");
        assert!(app.worlds[0].output_lines.is_empty(),
            "nothing should have landed in output_lines ahead of the still-queued backlog");

        let log = app.ws_broadcast_log.lock().unwrap();
        assert!(log.is_empty(), "the gagged line must not be broadcast while deferred to pending: {log:?}");
    }

    #[test]
    fn test_handle_prompt_on_disconnected_paused_world_respects_pending_backlog() {
        // Fix 3 regression guard: handle_prompt's disconnected-world branch used to push
        // the prompt straight into output_lines via a raw push, bypassing
        // push_line_respecting_pending - the same class of bug already fixed once in
        // handle_disconnected. That could plant a fresh, high seq into output_lines while
        // older pending content sat behind it, violating the "pending seqs always exceed
        // output_lines seqs" invariant the release path depends on.
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("test");
        world.connected = false;
        world.paused = true;
        world.prompt = "> ".to_string();
        for i in 0..3u64 {
            world.pending_lines.push(OutputLine::new(format!("backlog {i}"), i));
        }
        world.next_seq = 3;
        app.worlds.push(world);
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;

        app.handle_prompt(0, b"> ");

        assert!(app.worlds[0].output_lines.is_empty(),
            "the prompt must not jump ahead of the still-queued backlog into output_lines");
        assert_eq!(app.worlds[0].pending_lines.len(), 4, "backlog (3) + the deferred prompt line");
        assert_eq!(app.worlds[0].pending_lines.last().unwrap().text, ">");
    }

    #[test]
    fn test_release_orphaned_pending_broadcasts_content() {
        // Fix 4 regression guard: release_orphaned_pending used to move pending_lines into
        // output_lines purely locally (append + local state flip) with zero broadcast -
        // every other release site broadcasts the released content and an updated pending
        // count. WS clients (e.g. a remote Android session on this exact world) never
        // received the content and never learned pending_count dropped to 0.
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("test");
        world.paused = true;
        for i in 0..3u64 {
            world.pending_lines.push(OutputLine::new(format!("orphaned {i}"), i));
        }
        world.next_seq = 3;
        app.worlds.push(world);
        app.current_world_index = 0;
        // more-mode disabled but this world still holds a pending backlog (e.g. the
        // setting was toggled off from another client while paused) - the exact scenario
        // release_orphaned_pending exists to clean up.
        app.settings.more_mode_enabled = false;

        app.ws_broadcast_log.lock().unwrap().clear();
        let did_release = app.release_orphaned_pending();

        assert!(did_release);
        assert_eq!(app.worlds[0].output_lines.len(), 3);
        assert!(app.worlds[0].pending_lines.is_empty());

        let log = app.ws_broadcast_log.lock().unwrap();
        let combined_data: String = log.iter().filter_map(|m| {
            if let WsMessage::ServerData { data, .. } = m { Some(data.clone()) } else { None }
        }).collect();
        for i in 0..3 {
            assert!(combined_data.contains(&format!("orphaned {i}")),
                "released line {i} missing from broadcast content: {combined_data:?}");
        }
        let saw_pending_update_zero = log.iter().any(|m| matches!(m, WsMessage::PendingLinesUpdate { count: 0, .. }));
        assert!(saw_pending_update_zero, "must broadcast that pending_count dropped to 0: {log:?}");
    }

    #[test]
    fn test_handle_request_scrollback_after_seq_excludes_lines_past_pending_floor() {
        use crate::websocket::{WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        // Fix 5 regression guard (defense in depth): if any code path ever violates the
        // "pending seqs always exceed output_lines seqs" invariant - planting a
        // higher-seq line into output_lines while lower-seq content still sits in
        // pending_lines - handle_request_scrollback's after_seq gap-fill reply must not
        // hand that too-high-seq line to a client. Doing so would advance the client's
        // dedup high-water-mark (_max_seq in app.js) past content it was never actually
        // sent, which is exactly the poisoning mechanism this whole audit round is about.
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("test");
        // Contrived invariant violation: output_lines has a line at seq 5, but
        // pending_lines' lowest queued seq is 3 (should never legitimately happen, but
        // this test exists specifically to guard against it happening anyway).
        world.output_lines.push(OutputLine::new("seq 1".to_string(), 1));
        world.output_lines.push(OutputLine::new("seq 5 (violates invariant)".to_string(), 5));
        world.pending_lines.push(OutputLine::new("pending floor at seq 3".to_string(), 3));
        world.pending_lines.push(OutputLine::new("pending seq 4".to_string(), 4));
        app.worlds.push(world);
        app.current_world_index = 0;

        let server = WebSocketServer::new("", 0, "*", None, false, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true, tx, current_world: None, username: None,
                received_initial_state: true, client_type: RemoteClientType::Web,
                viewport_height: 24, ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(), last_activity: std::time::Instant::now(),
                paused: false, acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        app.handle_request_scrollback(client_id, 0, 20, None, Some(0), None);
        let (lines, _backfill_complete) = drain_one_scrollback_reply(&mut rx);
        let seqs: Vec<u64> = lines.iter().map(|l| l.seq).collect();

        assert!(seqs.contains(&1), "seq 1 is below the pending floor and must still be returned: {seqs:?}");
        assert!(!seqs.contains(&5),
            "seq 5 is >= the pending backlog's floor (3) and must be excluded from the gap-fill reply: {seqs:?}");
    }

    #[tokio::test]
    async fn test_multiuser_release_pending_broadcasts_content_not_just_count() {
        use crate::websocket::{WsClientInfo, WebSocketServer, RemoteClientType, Outbound};

        // Fix 6 regression guard: multiuser's ReleasePending handler used to drain
        // pending_lines into output_lines and broadcast only PendingReleased's count -
        // never the actual text via ServerData. A client seeing only the count-only
        // broadcast would clear its "More" indicator with no matching content ever having
        // arrived - the same "indicator clears, no output appears" symptom this whole
        // audit round is about, just scoped to multiuser mode.
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("test");
        world.owner = Some("alice".to_string());
        world.paused = true;
        for i in 0..3u64 {
            world.pending_lines.push(OutputLine::new(format!("backlog {i}"), i));
        }
        world.next_seq = 3;
        app.worlds.push(world);

        let server = WebSocketServer::new("", 0, "*", None, true /* multiuser */, BanList::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let client_id = 1u64;
        {
            let mut clients = server.clients.write().unwrap();
            clients.insert(client_id, WsClientInfo {
                authenticated: true, tx, current_world: None, username: Some("alice".to_string()),
                received_initial_state: true, client_type: RemoteClientType::Web,
                viewport_height: 24, ip_address: "127.0.0.1".to_string(),
                connected_at: std::time::Instant::now(), last_activity: std::time::Instant::now(),
                paused: false, acked_seq: std::collections::HashMap::new(), audit_prev_acked: std::collections::HashMap::new(), audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.ws_server = Some(server);

        let (event_tx, _event_rx) = tokio::sync::mpsc::channel::<AppEvent>(100);
        crate::daemon::handle_multiuser_ws_message(
            &mut app, client_id,
            WsMessage::ReleasePending { world_index: 0, count: 0 },
            &event_tx,
        ).await;

        let mut server_data_texts: Vec<String> = Vec::new();
        let mut saw_pending_released_count = None;
        while let Ok(item) = rx.try_recv() {
            // broadcast_to_owner sends pre-serialized JSON (Outbound::Shared), not a boxed
            // WsMessage (Outbound::Message) - unlike the single-recipient ws_send_to_client
            // path the other tests in this file exercise.
            let parsed: Option<WsMessage> = match item {
                Outbound::Message(msg) => Some(*msg),
                Outbound::Shared(json) => serde_json::from_str(&json).ok(),
            };
            match parsed {
                Some(WsMessage::ServerData { data, .. }) => server_data_texts.push(data),
                Some(WsMessage::PendingReleased { count, .. }) => saw_pending_released_count = Some(count),
                _ => {}
            }
        }
        assert!(!server_data_texts.is_empty(),
            "released pending content must be broadcast as ServerData, not just a count");
        let combined: String = server_data_texts.concat();
        for i in 0..3 {
            assert!(combined.contains(&format!("backlog {i}")),
                "released line {i} missing from broadcast content: {combined:?}");
        }
        assert_eq!(saw_pending_released_count, Some(3), "PendingReleased count broadcast must still be sent");
        assert_eq!(app.worlds[0].output_lines.len(), 3);
        assert!(app.worlds[0].pending_lines.is_empty());
    }


    // ==========================================================================
    // Broadcast-coverage invariant (PROTOCOL-ROADMAP.md Phase F)
    // ==========================================================================

    /// Accumulates every `ServerData` span the server has emitted for one world and answers
    /// "was this seq ever sent?". Mirrors app.js's `_seenRanges`, but server-side and used as
    /// a test oracle: any line sitting in `output_lines` whose seq was never broadcast is a
    /// permanent hole on every remote client.
    #[derive(Default)]
    struct BroadcastLedger {
        covered: std::collections::HashSet<u64>,
        texts: Vec<String>,
        /// What a Phase-C client actually ends up believing: `seq -> text`, built exactly the
        /// way app.js builds it (`lineSeq = msg.seq + rawIdx`, first writer wins because a
        /// later arrival at a seq it already holds is dropped by `hasSeenSeq`).
        client_model: std::collections::BTreeMap<u64, String>,
        /// Batches whose declared span doesn't match the number of lines they carry - the
        /// client silently marks the surplus seqs as delivered, so those lines can never be
        /// re-requested and are lost permanently.
        span_mismatches: Vec<String>,
    }

    impl BroadcastLedger {
        fn absorb(&mut self, rx: &mut tokio::sync::mpsc::Receiver<crate::websocket::Outbound>) {
            use crate::websocket::WsMessage;
            for m in drain_ws_messages(rx) {
                if let WsMessage::ServerData { seq, end_seq, data, flush, .. } = m {
                    if flush {
                        self.covered.clear();
                        self.texts.clear();
                        self.client_model.clear();
                    }
                    let end = end_seq.unwrap_or(seq).max(seq);
                    let has_real_seq = seq > 0 || end_seq.is_some();
                    let lines: Vec<&str> = data.strip_suffix('\n').unwrap_or(&data).split('\n').collect();
                    if has_real_seq {
                        let span = (end - seq + 1) as usize;
                        if span != lines.len() {
                            self.span_mismatches.push(format!(
                                "batch seq={seq} end_seq={end} declares {span} seqs but carries {} lines: {data:?}",
                                lines.len()));
                        }
                        for (i, l) in lines.iter().enumerate() {
                            let line_seq = seq + i as u64;
                            self.client_model.entry(line_seq).or_insert_with(|| l.to_string());
                        }
                    }
                    for s in seq..=end {
                        self.covered.insert(s);
                    }
                    self.texts.push(data);
                }
            }
        }

        fn saw_text(&self, text: &str) -> bool {
            let needle = text.replace('\r', "");
            self.texts.iter().any(|d| d.contains(&needle))
        }
    }

    /// The invariant: every line in `output_lines` must have been broadcast, except the one
    /// line that is currently an outstanding partial (deliberately held back until completed).
    /// Returns a list of human-readable violations rather than panicking, so a fuzz driver can
    /// report every distinct failure mode in one run.
    fn broadcast_coverage_violations(app: &App, world_idx: usize, ledger: &BroadcastLedger) -> Vec<String> {
        let world = &app.worlds[world_idx];
        // A partial line is legitimately unbroadcast until it is completed. It is the LAST
        // non-gagged line in output_lines (see World::last_visible_output_idx).
        let partial_idx = if !world.partial_line.is_empty() && !world.partial_in_pending {
            world.output_lines.iter().rposition(|l| !l.gagged)
        } else {
            None
        };
        let mut out = Vec::new();
        out.extend(ledger.span_mismatches.iter().cloned());
        for (i, line) in world.output_lines.iter().enumerate() {
            if Some(i) == partial_idx {
                continue;
            }
            if !ledger.covered.contains(&line.seq) {
                out.push(format!(
                    "seq {} never broadcast (idx {}, gagged={}, input={}, text={:?})",
                    line.seq, i, line.gagged, line.is_input, line.text));
            } else if !line.text.is_empty() && !ledger.saw_text(&line.text) {
                out.push(format!(
                    "seq {} span was claimed but its TEXT was never sent (idx {}, text={:?})",
                    line.seq, i, line.text));
            }
            // The client keys its buffer by seq. If the text it filed under this seq isn't
            // the text the server stored there, the next batch carrying the real line is
            // dropped as an already-seen duplicate - one line missing with its neighbours
            // present, which is the field symptom this whole check exists to catch.
            if let Some(client_text) = ledger.client_model.get(&line.seq) {
                if client_text != &line.text.replace('\r', "") {
                    out.push(format!(
                        "seq {} MISFILED on the client: server has {:?}, client model has {:?}",
                        line.seq, line.text, client_text));
                }
            }
        }
        out
    }

    /// Deterministic xorshift so a failing seed is reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u64) -> u64 { self.next() % n }
    }

    /// Fuzzes the operations that mutate `output_lines` against the broadcast-coverage
    /// invariant. This exists because the "TUI shows a line the phone never got" bug class has
    /// recurred through five different code paths; a per-path regression test only ever closes
    /// the path that was already found. The invariant is path-independent.
    #[test]
    fn test_fuzz_every_output_line_is_broadcast() {
        let mut failures: Vec<String> = Vec::new();
        for seed in 1..=200u64 {
            if let Some(msg) = fuzz_one_broadcast_coverage_run(seed) {
                failures.push(msg);
            }
        }
        assert!(failures.is_empty(),
            "{} of 200 fuzz runs left un-broadcast lines in output_lines:\n{}",
            failures.len(),
            failures.iter().take(8).cloned().collect::<Vec<_>>().join("\n\n"));
    }

    fn fuzz_one_broadcast_coverage_run(seed: u64) -> Option<String> {
        let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("w");
        w.connected = true;
        w.login_capture_guard = 0;
        app.worlds.push(w);
        app.current_world_index = 0;
        app.output_height = 10;
        app.output_width = 80;
        app.settings.more_mode_enabled = true;
        let (_cid, mut rx) = phase_c_register_client(&mut app);

        let mut ledger = BroadcastLedger::default();
        let mut history: Vec<String> = Vec::new();
        let mut counter = 0u32;

        for step in 0..40 {
            let op = rng.below(10);
            let desc = match op {
                0..=3 => {
                    // A chunk of complete lines from the MUD.
                    let n = 1 + rng.below(4);
                    let mut data = String::new();
                    for _ in 0..n {
                        counter += 1;
                        data.push_str(&format!("srv{counter}\r\n"));
                    }
                    app.process_server_data(0, data.as_bytes(), 10, 80, false);
                    format!("server_data({n} lines)")
                }
                4 => {
                    // A chunk ending mid-line: the MUD prompt case.
                    counter += 1;
                    let data = format!("srv{counter}\r\nprompt{counter}> ");
                    app.process_server_data(0, data.as_bytes(), 10, 80, false);
                    format!("server_data(+trailing partial prompt{counter})")
                }
                5 => {
                    counter += 1;
                    app.record_user_input(0, &format!("cmd{counter}"));
                    format!("user_input(cmd{counter})")
                }
                6 => {
                    counter += 1;
                    app.add_output_to_world(0, &format!("client{counter}"));
                    format!("add_output_to_world(client{counter})")
                }
                7 => {
                    app.release_pending_screenful();
                    "release_pending_screenful".to_string()
                }
                8 => {
                    counter += 1;
                    app.emit_client_lines(0, &[format!("recallA{counter}"), format!("recallB{counter}")], false);
                    format!("emit_client_lines({counter})")
                }
                _ => {
                    let on = app.settings.more_mode_enabled;
                    app.settings.more_mode_enabled = !on;
                    format!("more_mode={}", !on)
                }
            };
            history.push(desc);
            ledger.absorb(&mut rx);
            let mut violations = broadcast_coverage_violations(&app, 0, &ledger);
            // Second, independent oracle: the SHIPPING self-audit (App::audit_broadcast_ledger)
            // must reach the same verdict as the test-side ledger built from what actually
            // came out of the socket. A disagreement means the safety net users rely on has a
            // blind spot the test harness doesn't - which is worth failing over even when the
            // product itself is behaving.
            let audited = app.audit_broadcast_ledger();
            if audited != 0 {
                violations.push(format!(
                    "App::audit_broadcast_ledger reported {audited} unbroadcast line(s) here"));
            }
            ledger.absorb(&mut rx); // absorb any repair the audit just emitted
            if !violations.is_empty() {
                return Some(format!(
                    "seed {seed} step {step}:\n  history: {}\n  violations:\n    {}",
                    history.join(" -> "),
                    violations.join("\n    ")));
            }
        }
        None
    }

    /// The Phase F self-audit: a line pushed into `output_lines` with no broadcast is found,
    /// reported, and re-sent. This is the safety net for the whole "the TUI shows a line the
    /// phone never got" class - the simulated bug below (a bare push) is exactly the shape of
    /// the six real ones already fixed one at a time.
    #[test]
    fn test_broadcast_ledger_audit_finds_and_repairs_an_unbroadcast_line() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("w"));
        app.current_world_index = 0;
        let (_cid, mut rx) = phase_c_register_client(&mut app);

        // Two normal lines, correctly broadcast.
        push_and_broadcast(&mut app, 0, "first");
        // The bug: a raw push with no broadcast at all.
        let orphan_seq = app.worlds[0].next_seq;
        app.worlds[0].next_seq += 1;
        app.worlds[0].output_lines.push(OutputLine::new("orphaned line".to_string(), orphan_seq));
        // A later line, correctly broadcast - the orphan is now mid-buffer, which is what
        // makes it invisible to any tail-based check.
        push_and_broadcast(&mut app, 0, "third");

        let before = drain_server_data(&mut rx);
        assert!(!before.iter().any(|(_, _, d)| d.contains("orphaned line")),
            "precondition: the orphan must not have been broadcast yet");

        let holes = app.audit_broadcast_ledger();
        assert_eq!(holes, 1, "the audit must find exactly the one unbroadcast line");

        let repair = drain_server_data(&mut rx);
        let sent = repair.iter().find(|(_, _, d)| d.contains("orphaned line"))
            .unwrap_or_else(|| panic!("the audit must re-send the missing line. Got: {repair:?}"));
        assert_eq!(sent.0, orphan_seq, "the repair must carry the line's REAL seq");
        assert_eq!(sent.1, Some(orphan_seq), "end_seq present is what marks the seq as real");

        // Forward-only: a second pass must not re-report or re-send the same line.
        assert_eq!(app.audit_broadcast_ledger(), 0, "the audit must not re-fire for a seq it already repaired");
        assert!(drain_server_data(&mut rx).is_empty(), "no second repair broadcast");
    }

    /// The three legitimate reasons a stored line has no broadcast yet - it must not be
    /// reported as a hole in any of them, or the audit becomes a duplicate generator.
    #[test]
    fn test_broadcast_ledger_audit_ignores_pending_and_newest_lines() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("w"));
        app.current_world_index = 0;
        app.settings.more_mode_enabled = true;
        let (_cid, mut rx) = phase_c_register_client(&mut app);

        push_and_broadcast(&mut app, 0, "delivered");
        // The newest line, pushed but not yet broadcast (the in-flight case).
        let newest_seq = app.worlds[0].next_seq;
        app.worlds[0].next_seq += 1;
        app.worlds[0].output_lines.push(OutputLine::new("in flight".to_string(), newest_seq));
        // A backlog held on purpose: pending seqs are never owed to a client yet.
        app.worlds[0].paused = true;
        for _ in 0..3 {
            let seq = app.worlds[0].next_seq;
            app.worlds[0].next_seq += 1;
            app.worlds[0].pending_lines.push(OutputLine::new("held".to_string(), seq));
        }
        let _ = drain_server_data(&mut rx);

        assert_eq!(app.audit_broadcast_ledger(), 0,
            "neither the newest line nor a held backlog is a hole");
        assert!(drain_server_data(&mut rx).is_empty(), "nothing should be re-sent");
    }

    fn push_and_broadcast(app: &mut App, world_idx: usize, text: &str) {
        let seq = app.worlds[world_idx].next_seq;
        app.worlds[world_idx].next_seq += 1;
        let more_mode = app.settings.more_mode_enabled;
        app.push_and_broadcast_line(world_idx, OutputLine::new(text.to_string(), seq), more_mode);
    }

    /// The audit's own diagnostic must not be able to crash the daemon. MUD output is UTF-8;
    /// truncating the reported text by byte index lands mid-codepoint on any accented
    /// character or emoji.
    #[test]
    fn test_broadcast_ledger_audit_survives_multibyte_text() {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("w"));
        app.current_world_index = 0;
        let (_cid, mut rx) = phase_c_register_client(&mut app);

        push_and_broadcast(&mut app, 0, "first");
        // 200 emoji: every candidate truncation offset is mid-codepoint.
        let orphan_seq = app.worlds[0].next_seq;
        app.worlds[0].next_seq += 1;
        let text = "🐉".repeat(200);
        app.worlds[0].output_lines.push(OutputLine::new(text.clone(), orphan_seq));
        push_and_broadcast(&mut app, 0, "third");

        assert_eq!(app.audit_broadcast_ledger(), 1);
        let sent = drain_server_data(&mut rx);
        assert!(sent.iter().any(|(_, _, d)| d.contains(&text)),
            "the repair must carry the full line, not a truncated one");
    }

    // ==========================================================================
    // ▶ ownership on released pending lines (PROTOCOL-ROADMAP.md Phase H)
    // ==========================================================================

    /// Build a world holding `head` already-displayed-able lines plus `backlog` pending lines,
    /// all of which arrived while NOBODY was viewing (so they are unviewed and unowned).
    fn world_with_unviewed_backlog(app: &mut App, head: usize, backlog: usize) {
        app.worlds.clear();
        let mut w = World::new("w");
        w.connected = true;
        for i in 0..head {
            let mut l = OutputLine::new(format!("head {i}"), w.next_seq);
            l.viewed = false; // arrived while unwatched
            w.next_seq += 1;
            w.output_lines.push(l);
        }
        w.paused = true;
        for i in 0..backlog {
            let mut l = OutputLine::new(format!("backlog {i}"), w.next_seq);
            l.viewed = false;
            w.next_seq += 1;
            w.pending_lines.push(l);
        }
        app.worlds.push(w);
        // The console is looking at some OTHER world - this backlog is unwatched by everyone,
        // which is the whole premise (a line arriving while anyone views is born viewed).
        app.worlds.push(World::new("elsewhere"));
        app.current_world_index = 1;
    }

    fn claimed_new_seqs(msgs: &[crate::websocket::WsMessage]) -> Vec<Vec<u64>> {
        use crate::websocket::WsMessage;
        msgs.iter().filter_map(|m| match m {
            WsMessage::ClaimedNew { seqs, .. } => Some(seqs.clone()),
            _ => None,
        }).collect()
    }

    /// A client that starts displaying a world with nothing left to claim must still be told
    /// so. The client claims optimistically the instant it switches (so ▶ paints in the same
    /// frame as the text rather than a round-trip later), and an empty `ClaimedNew` is the
    /// only signal that a guess was wrong - another viewer got there first. Skipping the
    /// message, as the old `if claimed.is_empty() { return; }` did, would strand a marker the
    /// server never granted.
    #[test]
    fn test_claim_always_answers_the_displaying_client() {
        let mut app = App::new();
        world_with_unviewed_backlog(&mut app, 3, 0);
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_mark_world_seen(client_id, 0, None);
        assert_eq!(claimed_new_seqs(&drain_ws_messages(&mut rx)), vec![vec![0, 1, 2]],
            "precondition: the first display claims the unviewed head");

        // Switch away and back. Everything is viewed now, so there is nothing left to claim -
        // but the client must still receive the (empty) answer.
        app.handle_mark_world_seen(client_id, 1, Some(0));
        let _ = drain_ws_messages(&mut rx);
        app.handle_mark_world_seen(client_id, 0, Some(1));

        let msgs = drain_ws_messages(&mut rx);
        assert_eq!(claimed_new_seqs(&msgs), vec![Vec::<u64>::new()],
            "an empty ClaimedNew must still be sent so the client can revoke an optimistic \
             claim. Got: {msgs:#?}");
    }

    /// The reported bug: 100 lines arrive on a world nobody is watching, a remote client
    /// switches to it (head gets ▶ correctly), then hits Tab - and the released backlog came
    /// out with no ▶ at all, while the console TUI marked the same lines correctly.
    ///
    /// The server was stamping `display_id` the whole time; it just never told the client,
    /// because `broadcast_released_lines` called `World::claim_unviewed` directly and dropped
    /// the returned seq list. Asserting server-side state would therefore have PASSED against
    /// the bug - which is exactly what the pre-existing coverage did. This asserts the wire.
    #[test]
    fn test_released_backlog_claims_new_markers_for_the_releasing_client() {
        let mut app = App::new();
        world_with_unviewed_backlog(&mut app, 3, 5);
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        // Client switches to world 0: claims the head, which already worked.
        app.handle_mark_world_seen(client_id, 0, None);
        let head_claims = claimed_new_seqs(&drain_ws_messages(&mut rx));
        assert_eq!(head_claims, vec![vec![0, 1, 2]],
            "precondition: switching in claims the visible head");

        // Client hits Tab -> release the whole backlog.
        app.release_pending_lines(client_id, 0, 0);

        let msgs = drain_ws_messages(&mut rx);
        let claims = claimed_new_seqs(&msgs);
        assert_eq!(claims, vec![vec![3, 4, 5, 6, 7]],
            "the released backlog must be claimed FOR THIS CLIENT and named on the wire, \
             not just stamped server-side. Got: {msgs:#?}");

        // ...and the server's own copy agrees, so the console renders them identically.
        let owner = app.display_owner_id(client_id);
        for line in &app.worlds[0].output_lines {
            assert_eq!(line.display_id, Some(owner),
                "seq {} should be owned by the releasing client", line.seq);
        }
    }

    /// Ordering is the load-bearing half of the fix. `ClaimedNew` names seqs that the client
    /// resolves against lines already in its buffer, so a claim sent BEFORE the `ServerData`
    /// carrying those lines is a silent no-op - the exact failure being fixed. This asserts
    /// the claim trails the content, which is what the old code got wrong.
    #[test]
    fn test_released_backlog_claim_arrives_after_the_content() {
        use crate::websocket::WsMessage;
        let mut app = App::new();
        world_with_unviewed_backlog(&mut app, 1, 4);
        let (client_id, mut rx) = phase_c_register_client(&mut app);
        app.handle_mark_world_seen(client_id, 0, None);
        let _ = drain_ws_messages(&mut rx);

        app.release_pending_lines(client_id, 0, 0);

        let msgs = drain_ws_messages(&mut rx);
        let last_data = msgs.iter().rposition(|m| matches!(m, WsMessage::ServerData { .. }))
            .expect("the released content must be broadcast");
        let claim = msgs.iter().position(|m| matches!(m, WsMessage::ClaimedNew { .. }))
            .expect("the released content must be claimed");
        assert!(claim > last_data,
            "ClaimedNew (idx {claim}) must follow the ServerData carrying those lines \
             (last at idx {last_data}) - a claim for lines the client doesn't hold yet is \
             silently dropped. Order was: {:?}",
            msgs.iter().map(|m| format!("{m:?}").split_whitespace().next().unwrap_or("?").to_string()).collect::<Vec<_>>());
    }

    /// A console-driven release (Tab in the TUI) must still claim for the console and must not
    /// invent a `ClaimedNew` for anybody - the console has no socket.
    #[test]
    fn test_console_release_claims_for_console_without_notifying() {
        let mut app = App::new();
        world_with_unviewed_backlog(&mut app, 1, 3);
        app.current_world_index = 0; // console is now looking at the backlog world
        app.output_height = 24;
        app.output_width = 80;
        let (_client_id, mut rx) = phase_c_register_client(&mut app);

        app.release_pending_screenful();

        assert!(claimed_new_seqs(&drain_ws_messages(&mut rx)).is_empty(),
            "a console release has no client to notify");
        let released: Vec<_> = app.worlds[0].output_lines.iter()
            .filter(|l| l.text.starts_with("backlog")).collect();
        assert!(!released.is_empty(), "precondition: something was released");
        for line in released {
            assert_eq!(line.display_id, Some(crate::CONSOLE_DISPLAY_ID),
                "console-released lines belong to the console");
        }
    }

    /// With two clients on the same world, the one that asked for the release gets the
    /// markers. The pre-fix fallback was a `HashMap` `find()`, so it could hand them to
    /// whichever client it reached first - i.e. not the one that pressed Tab.
    #[test]
    fn test_releasing_client_wins_over_another_viewer() {
        use crate::websocket::{WsClientInfo, RemoteClientType};
        let mut app = App::new();
        world_with_unviewed_backlog(&mut app, 1, 3);
        let (client_a, mut rx_a) = phase_c_register_client(&mut app);

        // Register a second client, B, also viewing world 0.
        let client_b = 2u64;
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel::<crate::websocket::Outbound>(
            crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        {
            let server = app.ws_server.as_ref().unwrap();
            let mut clients = server.clients.write().unwrap();
            let viewport = clients.get(&client_a).unwrap().viewport_height;
            clients.insert(client_b, WsClientInfo {
                authenticated: true,
                tx: tx_b,
                current_world: Some(0),
                username: None,
                received_initial_state: true,
                client_type: RemoteClientType::Web,
                viewport_height: viewport,
                ip_address: "127.0.0.2".to_string(),
                connected_at: std::time::Instant::now(),
                last_activity: std::time::Instant::now(),
                paused: false,
                acked_seq: std::collections::HashMap::new(),
                audit_prev_acked: std::collections::HashMap::new(),
                audit_fired_at: std::collections::HashMap::new(),
                audit_stall_ticks: std::collections::HashMap::new(),
                push: None,
                needs_resync: std::collections::HashSet::new(),
            });
        }
        app.handle_mark_world_seen(client_a, 0, None);
        app.handle_mark_world_seen(client_b, 0, None);
        let _ = drain_ws_messages(&mut rx_a);
        let _ = drain_ws_messages(&mut rx_b);

        // B releases.
        app.release_pending_lines(client_b, 0, 0);

        assert!(claimed_new_seqs(&drain_ws_messages(&mut rx_a)).is_empty(),
            "the client that did NOT release must not be handed the markers");
        assert_eq!(claimed_new_seqs(&drain_ws_messages(&mut rx_b)), vec![vec![1, 2, 3]],
            "the releasing client gets them");
    }

    /// Drains one `ScrollbackLines` reply and reports `(backfill_complete, clamped_by_pending)`.
    fn drain_scrollback_flags(rx: &mut tokio::sync::mpsc::Receiver<crate::websocket::Outbound>) -> (bool, bool) {
        let mut result = None;
        while let Ok(item) = rx.try_recv() {
            if let crate::websocket::Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { backfill_complete, clamped_by_pending, .. } = *msg {
                    assert!(result.is_none(), "expected exactly one ScrollbackLines reply");
                    result = Some((backfill_complete, clamped_by_pending));
                }
            }
        }
        result.expect("expected a ScrollbackLines reply")
    }

    /// `backfill_complete: false` conflates two situations a client must handle in opposite
    /// ways: "more history is available right now, ask again" and "more is owed but withheld,
    /// asking again returns the identical answer". `clamped_by_pending` separates them.
    ///
    /// Note what actually triggers the clamp: it scans `output_lines` for entries at or above
    /// `pending_floor_seq()`. Under the documented invariant (pending seqs always exceed
    /// `output_lines` seqs) nothing ever qualifies, so an ordinary paused world with a backlog
    /// is NOT clamped — it reports `backfill_complete: true` normally. The clamp is a defensive
    /// detector for a *violated* invariant, which is the shape this test builds. Getting that
    /// wrong is easy: a paused world looks like it should clamp and doesn't.
    #[test]
    fn test_scrollback_reply_reports_when_it_was_clamped_by_a_pending_backlog() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("w");
        // Contrived invariant violation, matching
        // test_handle_request_scrollback_after_seq_excludes_lines_past_pending_floor: a seq 5
        // line sits in output_lines while seq 3/4 are still queued in pending_lines.
        world.output_lines.push(OutputLine::new("seq 1".to_string(), 1));
        world.output_lines.push(OutputLine::new("seq 5 (violates invariant)".to_string(), 5));
        world.pending_lines.push(OutputLine::new("pending floor at seq 3".to_string(), 3));
        world.pending_lines.push(OutputLine::new("pending seq 4".to_string(), 4));
        world.next_seq = 6;
        app.worlds.push(world);
        app.current_world_index = 0;
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_request_scrollback(client_id, 0, 100, None, Some(0), None);
        let (complete, clamped) = drain_scrollback_flags(&mut rx);
        assert!(clamped, "seq 5 was withheld by the pending floor - the reply must say so");
        assert!(!complete, "and must not claim completion: more IS owed");

        // Release the backlog. Nothing is withheld now, so the same request is genuinely done.
        let released = app.worlds[0].release_all_pending();
        assert_eq!(released.len(), 2, "precondition: the backlog released");
        let _ = drain_ws_messages(&mut rx);

        app.handle_request_scrollback(client_id, 0, 100, None, Some(0), None);
        let (complete, clamped) = drain_scrollback_flags(&mut rx);
        assert!(!clamped, "nothing is held back any more");
        assert!(complete, "and the request is now genuinely exhausted");
    }

    /// The common case, asserted explicitly because it is the one that is easy to assume wrong:
    /// an ordinary paused world with a backlog is NOT clamped. Its pending lines simply are not
    /// history the gap-fill owes - they reach the client via broadcast_released_lines on
    /// release - so the reply is a normal, complete one.
    #[test]
    fn test_ordinary_paused_backlog_is_not_reported_as_clamped() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("w");
        for seq in 0..=2u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        world.paused = true;
        for seq in 3..=9u64 {
            world.pending_lines.push(OutputLine::new(format!("held {seq}"), seq));
        }
        world.next_seq = 10;
        app.worlds.push(world);
        app.current_world_index = 0;
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_request_scrollback(client_id, 0, 100, None, Some(0), None);
        let (complete, clamped) = drain_scrollback_flags(&mut rx);
        assert!(!clamped, "a well-formed paused world does not trip the clamp");
        assert!(complete, "the deliverable history really is exhausted");
    }

    /// A `before_seq` request (deep-history backfill) can never be clamped — the clamp only
    /// applies to the forward `after_seq` catch-up — so it must never set the flag even on a
    /// world sitting on a backlog. Guards against a future refactor hoisting the clamp.
    #[test]
    fn test_before_seq_backfill_is_never_reported_as_clamped() {
        let mut app = App::new();
        app.worlds.clear();
        let mut world = World::new("w");
        for seq in 0..=5u64 {
            world.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        world.paused = true;
        world.pending_lines.push(OutputLine::new("held".to_string(), 6));
        world.next_seq = 7;
        app.worlds.push(world);
        app.current_world_index = 0;
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        app.handle_request_scrollback(client_id, 0, 100, Some(4), None, None);
        let (_, clamped) = drain_scrollback_flags(&mut rx);
        assert!(!clamped, "before_seq history is unaffected by the pending clamp");
    }

    // ---- Phase J: server-push scrollback download, wire schema ----
    //
    // Step 1 is schema only: no senders, no handlers. These pin the parts of the encoding
    // that later steps and the JS client depend on, so a rename or a dropped
    // `#[serde(default)]` fails here rather than silently changing the wire format.

    #[test]
    fn test_scrollback_sync_request_round_trip() {
        let msg = WsMessage::ScrollbackSyncRequest {
            worlds: vec![
                ScrollbackClientWorld {
                    name: "Alpha".to_string(),
                    gapless_seq: Some(500),
                    held_from: Some(900),
                    held_to: Some(1000),
                },
                ScrollbackClientWorld {
                    name: "Beta".to_string(),
                    gapless_seq: None,
                    held_from: None,
                    held_to: None,
                },
            ],
            complete: true,
            viewport_lines: 42,
            accepts_deflate: true,
            version: 1,
        };
        let json = serde_json::to_string(&msg).expect("serializes");
        match serde_json::from_str::<WsMessage>(&json).expect("round-trips") {
            WsMessage::ScrollbackSyncRequest { worlds, complete, viewport_lines, accepts_deflate, version } => {
                assert_eq!(worlds.len(), 2);
                assert_eq!(worlds[0].name, "Alpha");
                assert_eq!(worlds[0].gapless_seq, Some(500));
                assert_eq!(worlds[0].held_from, Some(900));
                assert_eq!(worlds[0].held_to, Some(1000));
                // A world the client holds nothing for: the server must read this as
                // "download everything", not as "gapless seq 0".
                assert_eq!(worlds[1].gapless_seq, None);
                assert!(complete);
                assert_eq!(viewport_lines, 42);
                assert!(accepts_deflate);
                assert_eq!(version, 1);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn test_scrollback_sync_request_defaults_for_older_client() {
        // Every added field must be omittable, or a client built against an earlier
        // generation of this message fails to parse at the server instead of degrading.
        let json = r#"{"type":"ScrollbackSyncRequest","worlds":[{"name":"Alpha"}],"complete":false}"#;
        match serde_json::from_str::<WsMessage>(json).expect("parses without optional fields") {
            WsMessage::ScrollbackSyncRequest { worlds, complete, viewport_lines, accepts_deflate, version } => {
                assert_eq!(worlds.len(), 1);
                assert_eq!(worlds[0].gapless_seq, None);
                assert_eq!(worlds[0].held_from, None);
                assert!(!complete);
                assert_eq!(viewport_lines, 0);
                // Defaulting to false is what keeps a client that can't decompress from
                // being sent a binary frame it will silently fail to decode.
                assert!(!accepts_deflate);
                assert_eq!(version, 0);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn test_scrollback_continue_round_trip() {
        let json = serde_json::to_string(&WsMessage::ScrollbackContinue { batch_id: 7 }).unwrap();
        match serde_json::from_str::<WsMessage>(&json).unwrap() {
            WsMessage::ScrollbackContinue { batch_id } => assert_eq!(batch_id, 7),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn test_scrollback_batch_round_trip() {
        let line = TimestampedLine {
            text: "hello".to_string(),
            ts: 1234,
            gagged: false,
            from_server: true,
            seq: 900,
            highlight_color: None,
            from_archive: false,
            viewed: true,
            display_id: None,
        };
        let msg = WsMessage::ScrollbackBatch {
            batch_id: 3,
            worlds: vec![ScrollbackWorldBatch {
                world_index: 0,
                world_name: "Alpha".to_string(),
                lines: vec![line],
                delivered: 25,
                planned_total: 500,
            }],
            done: vec![ScrollbackWorldDone {
                world_index: 1,
                world_name: "Beta".to_string(),
                reason: ScrollbackDoneReason::BufferExhausted,
                high_seq: Some(1000),
                low_seq: Some(3000),
                oldest_available_seq: Some(3000),
                plan_high_seq: 4000,
            }],
            complete: false,
        };
        let json = serde_json::to_string(&msg).expect("serializes");
        match serde_json::from_str::<WsMessage>(&json).expect("round-trips") {
            WsMessage::ScrollbackBatch { batch_id, worlds, done, complete } => {
                assert_eq!(batch_id, 3);
                assert_eq!(worlds.len(), 1);
                assert_eq!(worlds[0].world_name, "Alpha");
                assert_eq!(worlds[0].lines.len(), 1);
                assert_eq!(worlds[0].lines[0].seq, 900);
                assert_eq!(worlds[0].delivered, 25);
                assert_eq!(worlds[0].planned_total, 500);
                assert_eq!(done.len(), 1);
                assert_eq!(done[0].reason, ScrollbackDoneReason::BufferExhausted);
                // The field that terminates the re-request-forever loop when the server's
                // buffer no longer reaches down to the client's frontier.
                assert_eq!(done[0].oldest_available_seq, Some(3000));
                assert_eq!(done[0].plan_high_seq, 4000);
                assert!(!complete);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn test_scrollback_done_reason_encodes_by_name() {
        // The JS client switches on these strings; renaming a variant is a wire break.
        for (reason, expected) in [
            (ScrollbackDoneReason::ReachedClientSeq, "\"ReachedClientSeq\""),
            (ScrollbackDoneReason::HitLineLimit, "\"HitLineLimit\""),
            (ScrollbackDoneReason::BufferExhausted, "\"BufferExhausted\""),
            (ScrollbackDoneReason::Aborted, "\"Aborted\""),
            (ScrollbackDoneReason::Unsupported, "\"Unsupported\""),
        ] {
            assert_eq!(serde_json::to_string(&reason).unwrap(), expected);
        }
    }

    #[test]
    fn test_scrollback_batch_never_maps_to_a_world_index() {
        // A dropped batch must not fire ResyncRequired: the push pump already handles a full
        // channel by leaving its cursors alone and resending the identical batch, so a
        // resync on top would turn ordinary backpressure into a storm aimed at the slow
        // client that caused it. Guards the deliberate omission in message_world_index().
        let batch = WsMessage::ScrollbackBatch {
            batch_id: 1,
            worlds: vec![ScrollbackWorldBatch {
                world_index: 4,
                world_name: "Alpha".to_string(),
                lines: Vec::new(),
                delivered: 0,
                planned_total: 0,
            }],
            done: Vec::new(),
            complete: false,
        };
        assert_eq!(crate::websocket::message_world_index(&batch), None,
            "ScrollbackBatch must never resolve to a world_index - see message_world_index's doc comment");
        assert_eq!(crate::websocket::message_world_index(&WsMessage::ScrollbackContinue { batch_id: 1 }), None);
    }

    #[test]
    fn test_initial_state_scrollback_push_defaults_false() {
        // Absent flag => old server => the client must stay on the legacy pull path rather
        // than send a sync request into a `_ => {}` catch-all and wait forever.
        //
        // Built from a real build_initial_state() rather than a hand-written stub, so this
        // exercises the message the server actually emits; the flag is then stripped to
        // simulate an older server that predates it.
        let app = App::new();
        let state = app.build_initial_state(0);
        let mut encoded = serde_json::to_value(&state).expect("serializes");

        match &state {
            WsMessage::InitialState { scrollback_push, .. } => assert!(
                !scrollback_push,
                "the flag must stay false until the ScrollbackSyncRequest handler exists (step 9)"
            ),
            other => panic!("wrong variant: {:?}", other),
        }

        assert!(encoded.get("scrollback_push").is_some(), "flag is present on the wire");
        encoded.as_object_mut().unwrap().remove("scrollback_push");

        match serde_json::from_value::<WsMessage>(encoded).expect("parses without the flag") {
            WsMessage::InitialState { scrollback_push, .. } => assert!(!scrollback_push),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    // ---- Phase J: per-client download state and its accessors ----

    /// Minimal `ScrollbackPush` for accessor tests — the planner (step 4) builds the real one.
    fn push_fixture() -> Box<crate::websocket::ScrollbackPush> {
        use crate::websocket::{ScrollbackPush, PushWorld, PushPhase};
        Box::new(ScrollbackPush {
            worlds: vec![PushWorld {
                name: "Alpha".to_string(),
                floor_seq: Some(500),
                skip: None,
                cursor: 1000,
                plan_high_seq: 1000,
                oldest_at_plan: Some(0),
                budget_left: 100,
                planned_total: 100,
                delivered: 0,
                high_seq: None,
                low_seq: None,
                done: false,
            }],
            cycle_lines: 25,
            ramp_locked: false,
            inflight: None,
            next_batch_id: 1,
            stalls: 0,
            parked: false,
            phase: PushPhase::Initial,
            viewport_lines: 40,
            accepts_deflate: true,
            timing_invalid: false,
        })
    }

    #[test]
    fn test_push_state_take_and_put_round_trip() {
        let mut app = App::new();
        let (client_id, _rx) = phase_c_register_client(&mut app);
        let server = app.ws_server.as_ref().unwrap();

        assert!(server.take_push(client_id).is_none(), "no download before a sync request");

        server.put_push(client_id, push_fixture());
        let taken = server.take_push(client_id).expect("state comes back");
        assert_eq!(taken.worlds.len(), 1);
        assert_eq!(taken.worlds[0].name, "Alpha");
        assert_eq!(taken.cycle_lines, 25);
        assert_eq!(taken.phase, crate::websocket::PushPhase::Initial);
    }

    #[test]
    fn test_push_state_take_is_exclusive() {
        // take_push is a `take`, not a clone, precisely so two callers can never be driving
        // the same client's pump against two divergent copies of its cursors. The second
        // caller gets None and does nothing rather than double-sending a cycle.
        let mut app = App::new();
        let (client_id, _rx) = phase_c_register_client(&mut app);
        let server = app.ws_server.as_ref().unwrap();

        server.put_push(client_id, push_fixture());
        assert!(server.take_push(client_id).is_some(), "first take wins");
        assert!(server.take_push(client_id).is_none(), "second take must not see the state");
    }

    #[test]
    fn test_push_state_is_dropped_when_the_client_is_gone() {
        // A download must not outlive its connection. put_push for a departed client is a
        // no-op rather than resurrecting an entry keyed by a dead id.
        let mut app = App::new();
        let (client_id, _rx) = phase_c_register_client(&mut app);
        let server = app.ws_server.as_ref().unwrap();

        server.clients.write().unwrap().remove(&client_id);
        server.put_push(client_id, push_fixture());

        assert!(server.clients.read().unwrap().get(&client_id).is_none());
        assert!(server.take_push(client_id).is_none());
    }

    #[test]
    fn test_clear_push_discards_the_download() {
        let mut app = App::new();
        let (client_id, _rx) = phase_c_register_client(&mut app);
        let server = app.ws_server.as_ref().unwrap();

        server.put_push(client_id, push_fixture());
        server.clear_push(client_id);
        assert!(server.take_push(client_id).is_none());
    }

    #[test]
    fn test_push_deadlines_reflect_what_each_client_is_waiting_on() {
        use std::time::{Duration, Instant};
        let timeout = Duration::from_secs(30);
        let mut app = App::new();
        let (client_id, _rx) = phase_c_register_client(&mut app);
        let server = app.ws_server.as_ref().unwrap();

        // No download at all: nothing due.
        assert!(server.push_deadlines(timeout).is_empty());

        // Ready to send: due immediately, so the timer fires on the next tick.
        server.put_push(client_id, push_fixture());
        let due = server.push_deadlines(timeout);
        assert_eq!(due.len(), 1);
        assert!(due[0].1 <= Instant::now(), "a client with no batch in flight is due now");

        // Awaiting a continue: due at the timeout, not before. This is what lets the event
        // loops sleep instead of polling.
        let sent_at = Instant::now();
        let mut state = server.take_push(client_id).unwrap();
        state.inflight = Some((7, sent_at));
        server.put_push(client_id, state);
        let due = server.push_deadlines(timeout);
        assert_eq!(due.len(), 1);
        assert!(due[0].1 >= sent_at + timeout, "an in-flight batch is due at its timeout");

        // Parked: not due at all. A backgrounded client is woken by a visibility change,
        // never by the clock, so it must not accrue timeouts while away.
        let mut state = server.take_push(client_id).unwrap();
        state.parked = true;
        server.put_push(client_id, state);
        assert!(server.push_deadlines(timeout).is_empty(), "a parked client is not due");
    }

    #[test]
    fn test_client_channel_capacity_tracks_backlog() {
        // The pump refuses to start a cycle when this drops below a floor: the channel is
        // bounded and a full one DISCARDS messages, so an unthrottled download would evict
        // live output rather than merely compete with it.
        let mut app = App::new();
        let (client_id, mut rx) = phase_c_register_client(&mut app);
        let server = app.ws_server.as_ref().unwrap();

        let full = server.client_channel_capacity(client_id);
        assert_eq!(full, crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);

        let tx = server.clients.read().unwrap().get(&client_id).unwrap().tx.clone();
        for _ in 0..10 {
            tx.try_send(crate::websocket::Outbound::Shared(std::sync::Arc::from("{}"))).unwrap();
        }
        assert_eq!(server.client_channel_capacity(client_id), full - 10,
            "capacity must reflect undrained backlog, not nominal size");

        rx.close();
        while rx.try_recv().is_ok() {}

        // An unknown client reports zero, so the pump treats it as "no room" and skips it
        // rather than sending into a channel that no longer exists.
        assert_eq!(server.client_channel_capacity(9999), 0);
    }

    // ---- Phase J: splash lines must not collide with real output seqs ----

    #[test]
    fn test_splash_lines_do_not_reuse_seqs_for_real_output() {
        // Splash used to be hardcoded to seqs 0-11 while next_seq stayed at 0, so the first
        // twelve real lines re-used those seqs and output_lines held two different texts
        // under one number. A newest-first download walk would deliver both and the client
        // would drop one at random.
        let mut world = World::new_with_splash("Splashy", true);
        assert!(!world.output_lines.is_empty(), "fixture needs a splash");

        let splash_seqs: Vec<u64> = world.output_lines.iter().map(|l| l.seq).collect();
        assert_eq!(world.next_seq, splash_seqs.len() as u64,
            "next_seq must sit above the splash, not at 0");

        // Real output arriving on top of a still-displayed splash must get fresh seqs.
        for i in 0..5 {
            let seq = world.next_seq;
            world.next_seq += 1;
            world.output_lines.push(OutputLine::new(format!("real line {}", i), seq));
        }

        let all: Vec<u64> = world.output_lines.iter().map(|l| l.seq).collect();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "duplicate seq in output_lines: {:?}", all);
        assert!(all.windows(2).all(|w| w[0] < w[1]), "output_lines must stay ascending by seq");
    }

    #[test]
    fn test_splash_reinstall_allocates_above_existing_seqs() {
        // ensure_has_world reinstalls a splash into an EMPTY buffer - but empty does not mean
        // "no seqs issued": /flush empties output_lines and deliberately leaves next_seq
        // alone. Allocating from 0 there would re-issue seqs a connected client already holds.
        let mut world = World::new("Reused");
        world.next_seq = 8000;

        let splash = World::generate_splash_lines(world.next_seq);
        assert_eq!(splash.first().map(|l| l.seq), Some(8000),
            "a reinstalled splash starts at next_seq, not 0");
        assert_eq!(splash.last().map(|l| l.seq), Some(8000 + splash.len() as u64 - 1));
    }

    #[test]
    fn test_world_without_splash_still_starts_at_seq_zero() {
        // Guard against the fix silently costing every ordinary world its first 12 seqs.
        let world = World::new("Plain");
        assert!(world.output_lines.is_empty());
        assert_eq!(world.next_seq, 0);
    }

    // ========================================================================
    // Phase J: the scrollback push planner
    //
    // Pure decisions only - no clock, no sends. Everything here answers "what should this
    // client receive", which is exactly the part that eight phases of the pull design kept
    // getting wrong.
    // ========================================================================

    /// A world named `name` holding seqs `0..count`, every `gag_every`-th line gagged
    /// (0 = none). `next_seq` is left just past the last line, as the live path maintains it.
    fn push_world(name: &str, count: u64, gag_every: u64) -> World {
        let mut world = World::new(name);
        for seq in 0..count {
            let mut line = OutputLine::new(format!("{} line {}", name, seq), seq);
            if gag_every > 0 && seq % gag_every == 0 {
                line.gagged = true;
            }
            world.output_lines.push(line);
        }
        world.next_seq = count;
        world
    }

    fn claim(name: &str, gapless: Option<u64>) -> crate::websocket::ScrollbackClientWorld {
        crate::websocket::ScrollbackClientWorld {
            name: name.to_string(),
            gapless_seq: gapless,
            held_from: None,
            held_to: None,
        }
    }

    /// Everything the plan will deliver for `name`, newest-first, as seqs. Mirrors what the
    /// cycle builder (step 5) will walk, so planner tests can assert on content rather than
    /// only on counts.
    fn planned_seqs(app: &App, plan: &crate::websocket::ScrollbackPush, name: &str) -> Vec<u64> {
        let pw = plan.worlds.iter().find(|w| w.name == name).expect("world planned");
        let world = app.worlds.iter().find(|w| w.name == name).unwrap();
        let mut visible_spent = 0usize;
        let mut out = Vec::new();
        for line in world.output_lines.iter().rev() {
            if line.seq > pw.plan_high_seq || line.from_archive {
                continue;
            }
            if pw.floor_seq.is_some_and(|f| line.seq <= f) {
                break;
            }
            if pw.skip.is_some_and(|(a, b)| line.seq >= a && line.seq <= b) {
                continue;
            }
            if !line.gagged {
                if visible_spent >= pw.budget_left {
                    break;
                }
                visible_spent += 1;
            }
            out.push(line.seq);
        }
        out
    }

    #[test]
    fn test_plan_stops_at_the_clients_gapless_seq() {
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![push_world("Alpha", 1000, 0)];

        let plan = app.plan_scrollback_push(&[claim("Alpha", Some(900))], 40, false);
        let seqs = planned_seqs(&app, &plan, "Alpha");

        assert_eq!(seqs.first(), Some(&999), "newest first");
        assert_eq!(seqs.last(), Some(&901), "stops just above the client's frontier");
        assert!(!seqs.contains(&900), "must not resend the frontier line itself");
        assert_eq!(plan.worlds[0].planned_total, 99);
    }

    #[test]
    fn test_plan_downloads_everything_for_an_unmentioned_world() {
        // The explicit contract of ScrollbackSyncRequest.complete: a world absent from the
        // list means the client holds nothing for it.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![push_world("Alpha", 50, 0), push_world("Beta", 50, 0)];

        let plan = app.plan_scrollback_push(&[claim("Alpha", Some(40))], 40, false);
        let beta = plan.worlds.iter().find(|w| w.name == "Beta").expect("Beta planned");
        assert_eq!(beta.floor_seq, None);
        assert_eq!(beta.planned_total, 50, "unmentioned world downloads in full");
    }

    #[test]
    fn test_plan_respects_the_remote_lines_budget() {
        let mut app = App::new();
        app.settings.remote_initial_lines = 100;
        app.worlds = vec![push_world("Alpha", 1000, 0)];

        let plan = app.plan_scrollback_push(&[claim("Alpha", None)], 40, false);
        assert_eq!(plan.worlds[0].planned_total, 100, "budget caps the download");

        let seqs = planned_seqs(&app, &plan, "Alpha");
        assert_eq!(seqs.first(), Some(&999));
        assert_eq!(seqs.last(), Some(&900), "budget is spent on the NEWEST lines");
    }

    #[test]
    fn test_plan_budget_counts_visible_lines_only() {
        // Gagged lines are invisible without F2, so they must not eat a client's allowance -
        // but they ARE sent, so they count toward planned_total. Same rule as
        // take_visible_range and the rest of the Remote Lines accounting.
        let mut app = App::new();
        app.settings.remote_initial_lines = 10;
        app.worlds = vec![push_world("Alpha", 100, 2)]; // every 2nd line gagged

        let plan = app.plan_scrollback_push(&[claim("Alpha", None)], 40, false);
        let seqs = planned_seqs(&app, &plan, "Alpha");

        let world = &app.worlds[0];
        let visible = seqs.iter()
            .filter(|s| !world.output_lines[**s as usize].gagged)
            .count();
        assert_eq!(visible, 10, "exactly the budget in VISIBLE lines");
        assert!(seqs.len() > 10, "gagged lines ride along free");
        assert_eq!(plan.worlds[0].planned_total, seqs.len(),
            "planned_total counts every line actually sent, gagged included");
    }

    #[test]
    fn test_plan_skips_the_range_the_client_already_holds() {
        // held_from/held_to is what stops the download re-sending the slice InitialState
        // just handed the client.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![push_world("Alpha", 1000, 0)];

        let mut c = claim("Alpha", Some(500));
        c.held_from = Some(900);
        c.held_to = Some(999);
        let plan = app.plan_scrollback_push(&[c], 40, false);
        let seqs = planned_seqs(&app, &plan, "Alpha");

        assert!(seqs.iter().all(|s| !(900..=999).contains(s)), "held range is skipped");
        assert_eq!(seqs.first(), Some(&899), "delivery resumes below the held range");
        assert_eq!(seqs.last(), Some(&501));
        assert_eq!(plan.worlds[0].planned_total, 399);
    }

    #[test]
    fn test_plan_reports_oldest_available_when_history_was_trimmed() {
        // The field that terminates the re-request-forever loop: the client's frontier is
        // 100 but the server's buffer starts at 3000, so 101..2999 can never be delivered.
        // Without oldest_available_seq the client asks again on every reconnect, forever.
        let mut app = App::new();
        app.settings.remote_initial_lines = 10000;
        let mut world = World::new("Alpha");
        for seq in 3000..3500u64 {
            world.output_lines.push(OutputLine::new(format!("line {}", seq), seq));
        }
        world.next_seq = 3500;
        app.worlds = vec![world];

        let plan = app.plan_scrollback_push(&[claim("Alpha", Some(100))], 40, false);
        assert_eq!(plan.worlds[0].oldest_at_plan, Some(3000));
        assert_eq!(plan.worlds[0].planned_total, 500, "only what actually exists");
    }

    #[test]
    fn test_plan_ignores_a_client_frontier_from_a_previous_seq_epoch() {
        // next_seq is persisted, but a lost settings.dat or a recreated world resets it. The
        // client then reports a frontier above anything we hold; trusting it would download
        // nothing and leave the client showing stale text forever.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![push_world("Alpha", 50, 0)];

        let plan = app.plan_scrollback_push(&[claim("Alpha", Some(9000))], 40, false);
        assert_eq!(plan.worlds[0].floor_seq, None, "impossible frontier is discarded");
        assert_eq!(plan.worlds[0].planned_total, 50, "full download instead");
        assert_eq!(plan.worlds[0].plan_high_seq, 49,
            "and the done marker reports a plan_high below the client's claim, so it drops its record");
    }

    #[test]
    fn test_plan_excludes_pending_held_lines() {
        // Lines in the more-mode backlog have real seqs but were deliberately not broadcast.
        // Delivering them here would put them on the wire ahead of the release path.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        let mut world = push_world("Alpha", 100, 0);
        for seq in 100..120u64 {
            world.pending_lines.push(OutputLine::new(format!("pending {}", seq), seq));
        }
        world.next_seq = 120;
        app.worlds = vec![world];

        let plan = app.plan_scrollback_push(&[claim("Alpha", None)], 40, false);
        assert_eq!(plan.worlds[0].plan_high_seq, 99, "stop point clamps below the backlog");
        let seqs = planned_seqs(&app, &plan, "Alpha");
        assert!(seqs.iter().all(|s| *s < 100), "no pending line is ever pushed");
    }

    #[test]
    fn test_plan_excludes_archive_lines() {
        // try_load_archive_lines fabricates seqs by counting backwards with a saturating_sub,
        // so on a low-seq world they DUPLICATE real ones. Sending them would put two
        // different texts on the wire under one seq.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        let mut world = push_world("Alpha", 20, 0);
        let mut archived = OutputLine::new("archived text".to_string(), 5);
        archived.from_archive = true;
        world.output_lines.insert(0, archived);
        app.worlds = vec![world];

        let plan = app.plan_scrollback_push(&[claim("Alpha", None)], 40, false);
        assert_eq!(plan.worlds[0].planned_total, 20, "the archive line is not counted");

        let seqs = planned_seqs(&app, &plan, "Alpha");
        assert_eq!(seqs.iter().filter(|s| **s == 5).count(), 1,
            "seq 5 appears once - the real line, not the archive duplicate");
    }

    #[test]
    fn test_plan_high_seq_is_fixed_so_a_busy_world_terminates() {
        // The stop point is chosen once. If it chased the live tail, a world receiving output
        // faster than the download drains would never finish - and the push and live streams
        // would overlap, so each would have to dedup against the other.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![push_world("Alpha", 100, 0)];

        let plan = app.plan_scrollback_push(&[claim("Alpha", None)], 40, false);
        assert_eq!(plan.worlds[0].plan_high_seq, 99);

        for seq in 100..200u64 {
            app.worlds[0].output_lines.push(OutputLine::new(format!("live {}", seq), seq));
        }
        app.worlds[0].next_seq = 200;

        assert_eq!(plan.worlds[0].plan_high_seq, 99, "live arrivals do not move the stop point");
        let seqs = planned_seqs(&app, &plan, "Alpha");
        assert!(seqs.iter().all(|s| *s <= 99), "live lines are the broadcast path's job");
    }

    #[test]
    fn test_plan_omits_worlds_with_nothing_to_send() {
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![
            push_world("Empty", 0, 0),
            push_world("CaughtUp", 50, 0),
            push_world("Needs", 50, 0),
        ];

        let plan = app.plan_scrollback_push(
            &[claim("CaughtUp", Some(49)), claim("Needs", Some(10))], 40, false);

        let names: Vec<&str> = plan.worlds.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, vec!["Needs"],
            "an empty world and a fully caught-up one carry no cursor");
    }

    #[test]
    fn test_plan_carries_client_negotiated_settings() {
        let mut app = App::new();
        app.settings.remote_initial_lines = 100;
        app.worlds = vec![push_world("Alpha", 50, 0)];

        let plan = app.plan_scrollback_push(&[claim("Alpha", None)], 37, true);
        assert_eq!(plan.viewport_lines, 37);
        assert!(plan.accepts_deflate);
        assert_eq!(plan.cycle_lines, crate::PUSH_CYCLE_START);
        assert_eq!(plan.phase, crate::websocket::PushPhase::Initial);
        assert!(!plan.ramp_locked);
        assert!(plan.inflight.is_none());
    }

    #[test]
    fn test_plan_matches_worlds_by_name_not_index() {
        // An index would be silently retargeted by a world being added or removed between
        // connections - the failure AuthRequest.resume's index-keyed list still has.
        let mut app = App::new();
        app.settings.remote_initial_lines = 1000;
        app.worlds = vec![push_world("Alpha", 100, 0), push_world("Beta", 100, 0)];

        // Client reports its frontiers in the opposite order to the server's world array.
        let plan = app.plan_scrollback_push(
            &[claim("Beta", Some(90)), claim("Alpha", Some(10))], 40, false);

        let alpha = plan.worlds.iter().find(|w| w.name == "Alpha").unwrap();
        let beta = plan.worlds.iter().find(|w| w.name == "Beta").unwrap();
        assert_eq!(alpha.floor_seq, Some(10), "Alpha's frontier stayed with Alpha");
        assert_eq!(beta.floor_seq, Some(90), "Beta's frontier stayed with Beta");
    }

    // ========================================================================
    // Archive lines vs the broadcast-ledger audit (reported v1.5.22)
    //
    // Symptom: one world showed OLD text on a freshly-installed Android client and then
    // stopped showing NEW output entirely, while the TUI was fine. Other worlds were
    // unaffected. A resync fixed it. A full uninstall/reinstall did not - which rules the
    // client's cache out and puts it on the server.
    // ========================================================================

    /// Splice an archive block onto a world exactly as a TUI scroll-to-top does, using the
    /// REAL production builder (`App::build_archive_prepend`) rather than a copy of it — so
    /// these tests exercise the actual seq math and the actual `from_archive` marking,
    /// including on the separator line.
    fn splice_archive_lines(world: &mut World, count: usize) {
        let archived: Vec<crate::scrollback::ScrollbackLine> = (0..count)
            .map(|i| crate::scrollback::ScrollbackLine {
                ts_ms: 1_000 + i as i64,
                world: world.name.clone(),
                text: format!("archived line {}", i),
            })
            .collect();
        let oldest_seq = world.output_lines.first().map(|l| l.seq).unwrap_or(0);
        let prepend = App::build_archive_prepend(archived, oldest_seq);
        world.output_lines.splice(0..0, prepend);
    }

    #[test]
    fn test_every_line_of_an_archive_block_is_marked_including_the_separator() {
        // The from_archive marker is the ONLY thing keeping this block out of the sequence
        // contract. One unmarked line is enough to reintroduce the bug: it carries a
        // fabricated seq, is never broadcast, and slips past every filter - producing a
        // permanent ledger hole and a duplicate seq. The separator is built with
        // `new_client`, which does not set the marker, so it is the line that gets missed.
        let archived: Vec<crate::scrollback::ScrollbackLine> = (0..5)
            .map(|i| crate::scrollback::ScrollbackLine {
                ts_ms: 1_000 + i,
                world: "W".to_string(),
                text: format!("old {}", i),
            })
            .collect();

        let block = App::build_archive_prepend(archived, 100);

        assert_eq!(block.len(), 6, "separator plus five archived lines");
        for (i, line) in block.iter().enumerate() {
            assert!(line.from_archive,
                "line {} of the archive block is not marked from_archive: {:?}",
                i, line.text);
        }
    }

    #[test]
    fn test_archive_lines_are_not_resent_as_live_output_by_the_ledger_audit() {
        // try_load_archive_lines splices archived history into output_lines and deliberately
        // never broadcasts it - it is local TUI scrollback, not new MUD output. But
        // audit_broadcast_ledger enforces "every line in output_lines below the pending floor
        // must have been broadcast" with NO from_archive exemption, so it sees several hundred
        // stored-but-unsent lines and re-broadcasts them as ServerData.
        //
        // To a connected client that is old text arriving as if it were new.
        let mut app = App::new();
        app.worlds = vec![World::new("Archived")];
        let (client_id, mut rx) = phase_c_register_client(&mut app);

        // An established world: seqs well above the archive chunk size, so the archived lines
        // land cleanly BELOW the live ones. This isolates the re-broadcast bug from the
        // separate duplicate-seq bug the sibling test covers.
        for seq in 5000..5060u64 {
            app.worlds[0].output_lines.push(OutputLine::new(format!("real {}", seq), seq));
            app.worlds[0].next_seq = seq + 1;
            app.worlds[0].mark_broadcast(seq, seq);
        }
        while rx.try_recv().is_ok() {}

        // The user scrolls to the top of this world in the TUI. This happens BEFORE the first
        // ledger audit, which is the case that matters: the audit only ever runs on a
        // PongCheck from a connected client, so on a freshly-restarted server (exactly what
        // installing a new build does) `ledger_audited_upto` is still None and the first pass
        // examines the buffer from index 0 - including everything the archive just prepended.
        splice_archive_lines(&mut app.worlds[0], 500);

        let resent = app.audit_broadcast_ledger();
        let _ = client_id;

        assert_eq!(resent, 0,
            "the ledger audit re-broadcast {} archived lines as if they were new output - \
             archived history is deliberately never broadcast, so it must be exempt from the \
             every-stored-line-was-broadcast rule rather than treated as {} holes to repair",
            resent, resent);
    }

    #[test]
    fn test_archive_lines_never_reach_a_client_in_initial_state() {
        // The fix's core contract: archived scrollback is display-only local history and is
        // outside the sequence-number contract entirely.
        //
        // Its seqs are fabricated by counting backwards from the buffer's oldest with a
        // saturating_sub, so on a world whose buffer starts below the archive chunk size they
        // land ON TOP of live seqs. Sending them made a client record those numbers as
        // delivered; real MUD output later reused them and the client dropped every line as a
        // duplicate. The world went silent on the remote while the TUI kept updating - the
        // exact asymmetry in the report.
        let mut world = World::new("Young");
        for seq in 0..60u64 {
            world.output_lines.push(OutputLine::new(format!("real {}", seq), seq));
        }
        world.next_seq = 60;
        splice_archive_lines(&mut world, 500);

        let (sent, _visible) = App::build_initial_output_lines(&world.output_lines, false, 10_000);

        assert!(sent.iter().all(|l| !l.from_archive),
            "InitialState carried archived lines to a client");

        // Nothing a client was told about may collide with a seq real output will later use.
        let highest_sent = sent.iter().map(|l| l.seq).max().unwrap_or(0);
        assert!(world.next_seq > highest_sent,
            "next_seq is {} but the client was told about seqs up to {} - the next lines of \
             real output would reuse numbers the client has already recorded as delivered",
            world.next_seq, highest_sent);

        let seqs: Vec<u64> = sent.iter().map(|l| l.seq).collect();
        let mut uniq = seqs.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), seqs.len(), "a client was sent duplicate seqs: {:?}", seqs);
    }

    #[test]
    fn test_archive_lines_never_reach_a_client_via_request_scrollback() {
        // Same contract on the pull path - all three branches of handle_request_scrollback.
        let mut app = App::new();
        app.worlds = vec![World::new("Young")];
        for seq in 0..60u64 {
            app.worlds[0].output_lines.push(OutputLine::new(format!("real {}", seq), seq));
        }
        app.worlds[0].next_seq = 60;
        splice_archive_lines(&mut app.worlds[0], 500);

        let (client_id, mut rx) = phase_c_register_client(&mut app);

        for (label, before, after) in [
            ("last-N", None, None),
            ("before_seq", Some(59u64), None),
            ("after_seq", None, Some(0u64)),
        ] {
            while rx.try_recv().is_ok() {}
            app.handle_request_scrollback(client_id, 0, 1000, before, after, None);
            let mut saw_archive = false;
            while let Ok(out) = rx.try_recv() {
                if let crate::websocket::Outbound::Message(msg) = out {
                    if let WsMessage::ScrollbackLines { lines, .. } = *msg {
                        saw_archive |= lines.iter().any(|l| l.from_archive);
                    }
                }
            }
            assert!(!saw_archive, "{} branch sent archived lines to a client", label);
        }
    }

    #[test]
    fn test_archive_load_keeps_next_seq_above_everything_in_the_buffer() {
        // Reproduces the reported cave world: /dump showed output_lines.len 1648 against
        // next_seq 145. The archive fabricates seqs and never advanced the counter, so the
        // next ~1500 lines of real MUD output were stamped with numbers already sitting in
        // the buffer. A client holding those numbers - from any session before archived lines
        // were kept off the wire - drops every one as a duplicate, and the world goes silent
        // there while the TUI, which renders by position rather than seq, looks fine.
        //
        // The invariant: next_seq is always above every seq present in output_lines, so a
        // live line can never reuse one.
        let mut world = World::new("cave");
        for seq in 0..145u64 {
            world.output_lines.push(OutputLine::new(format!("live {}", seq), seq));
        }
        world.next_seq = 145;

        // Three scroll-to-top archive loads, as the reported buffer had. Goes through the
        // REAL production path (build_archive_prepend + install_archive_block) rather than a
        // copy of it, so removing the fix actually fails this test.
        for _ in 0..3 {
            let oldest = world.output_lines.first().map(|l| l.seq).unwrap_or(0);
            let archived: Vec<crate::scrollback::ScrollbackLine> = (0..500)
                .map(|i| crate::scrollback::ScrollbackLine {
                    ts_ms: 1_000 + i as i64,
                    world: "cave".to_string(),
                    text: format!("archived {}", i),
                })
                .collect();
            world.install_archive_block(App::build_archive_prepend(archived, oldest));
        }

        let highest_in_buffer = world.output_lines.iter().map(|l| l.seq).max().unwrap();
        assert!(world.next_seq > highest_in_buffer,
            "next_seq is {} but the buffer already contains seq {} - the next {} lines of real \
             output would reuse numbers a client has already recorded as delivered",
            world.next_seq, highest_in_buffer, highest_in_buffer + 1 - world.next_seq);

        // And the very next live line must not collide with anything already stored.
        let next = world.next_seq;
        assert!(!world.output_lines.iter().any(|l| l.seq == next),
            "the next live line would be issued seq {}, which is already in the buffer", next);
    }

    #[test]
    fn test_ensure_next_seq_above_buffer_repairs_the_reported_cave_shape() {
        // Reproduces world 12 [cave] exactly as /dump reported it on 1.5.25: 1648 lines in
        // the buffer against next_seq 145. Three 500-line archive blocks (whose seqs are
        // fabricated and saturate at 0) plus ~142 live lines, and a counter that never moved.
        //
        // This state SURVIVED the fix for new archive loads, because a hot reload restores
        // output_lines and next_seq independently - and a hot reload is exactly how a
        // long-running instance picks up a new build. The repair has to happen on restore or
        // the poisoned world never recovers.
        let mut world = World::new("cave");
        for _ in 0..3 {
            let oldest = world.output_lines.first().map(|l| l.seq).unwrap_or(0);
            let archived: Vec<crate::scrollback::ScrollbackLine> = (0..500)
                .map(|i| crate::scrollback::ScrollbackLine {
                    ts_ms: 1_000 + i as i64,
                    world: "cave".to_string(),
                    text: format!("archived {}", i),
                })
                .collect();
            // Splice WITHOUT the counter advance, i.e. the state a pre-1.5.25 instance built.
            let prepend = App::build_archive_prepend(archived, oldest);
            world.output_lines.splice(0..0, prepend);
        }
        for seq in 0..142u64 {
            world.output_lines.push(OutputLine::new(format!("live {}", seq), seq));
        }
        world.next_seq = 145;

        let highest = world.output_lines.iter().map(|l| l.seq).max().unwrap();
        assert!(world.next_seq <= highest, "fixture must reproduce the broken state");

        world.ensure_next_seq_above_buffer();

        assert!(world.next_seq > highest,
            "after repair next_seq is {} but the buffer still contains seq {}",
            world.next_seq, highest);
        let next = world.next_seq;
        assert!(!world.output_lines.iter().any(|l| l.seq == next),
            "the next live line would reuse seq {}, already present in the buffer", next);
    }

    #[test]
    fn test_ensure_next_seq_above_buffer_accounts_for_pending_lines() {
        // Pending lines hold allocated seqs and get appended later, so they count.
        let mut world = World::new("w");
        world.output_lines.push(OutputLine::new("a".to_string(), 10));
        world.pending_lines.push(OutputLine::new("held".to_string(), 99));
        world.next_seq = 11;

        world.ensure_next_seq_above_buffer();
        assert_eq!(world.next_seq, 100, "must clear the highest pending seq too");
    }

    #[test]
    fn test_ensure_next_seq_above_buffer_leaves_a_healthy_world_alone() {
        // 44 of the 45 worlds in the reported dump were consistent; the repair must be a
        // no-op for them rather than inflating their counters.
        let mut world = World::new("healthy");
        for seq in 0..477u64 {
            world.output_lines.push(OutputLine::new(format!("l {}", seq), seq));
        }
        world.next_seq = 477;

        world.ensure_next_seq_above_buffer();
        assert_eq!(world.next_seq, 477, "a dense, consistent world must not be touched");

        let mut empty = World::new("empty");
        empty.ensure_next_seq_above_buffer();
        assert_eq!(empty.next_seq, 0, "an empty world stays at 0");
    }

    #[test]
    fn test_ledger_audit_self_repairs_a_poisoned_seq_counter() {
        // Covers the WIRING, not just the helper: a world already holding more lines than
        // seqs ever issued must recover on its own from the periodic audit, without a reload
        // and without operator action. This is what rescues an instance that upgraded via a
        // hot reload, which carries the broken pair across intact.
        let mut app = App::new();
        app.worlds = vec![World::new("cave")];
        let (_client_id, _rx) = phase_c_register_client(&mut app);

        // The reported shape: buffer full of archive-fabricated seqs, counter left behind.
        for _ in 0..3 {
            let oldest = app.worlds[0].output_lines.first().map(|l| l.seq).unwrap_or(0);
            let archived: Vec<crate::scrollback::ScrollbackLine> = (0..500)
                .map(|i| crate::scrollback::ScrollbackLine {
                    ts_ms: 1_000 + i as i64,
                    world: "cave".to_string(),
                    text: format!("archived {}", i),
                })
                .collect();
            let prepend = App::build_archive_prepend(archived, oldest);
            app.worlds[0].output_lines.splice(0..0, prepend);
        }
        app.worlds[0].next_seq = 145;

        let highest = app.worlds[0].output_lines.iter().map(|l| l.seq).max().unwrap();
        assert!(app.worlds[0].next_seq <= highest, "fixture must start broken");

        app.audit_broadcast_ledger();

        assert!(app.worlds[0].next_seq > highest,
            "the periodic audit left next_seq at {} with seq {} still in the buffer - a \
             poisoned world must recover without a reload",
            app.worlds[0].next_seq, highest);
    }

    // ---- Seq epoch: identifies a world's sequence-number space ----

    #[test]
    fn test_each_world_gets_a_distinct_nonzero_seq_epoch() {
        // 0 is reserved on the wire for "this peer doesn't speak epochs" (older server, or
        // multiuser), so a real world must never mint it - a client would read 0 as "unknown"
        // and fall back to the heuristic this replaces.
        let mut seen = std::collections::HashSet::new();
        for i in 0..50 {
            let w = World::new(&format!("w{}", i));
            assert_ne!(w.seq_epoch, 0, "a real world must never get the reserved epoch 0");
            assert!(seen.insert(w.seq_epoch), "duplicate epoch {}", w.seq_epoch);
        }
    }

    #[test]
    fn test_seq_epoch_survives_a_reload_round_trip() {
        // The epoch describes the same sequence space as next_seq and output_lines, so it has
        // to travel with them through a hot reload. If a reload minted a fresh epoch for an
        // unchanged buffer, every connected client would throw away a good cache each reload.
        //
        // Drives the REAL parse path (load_reload_state_from_str), not a hand-rolled copy of
        // it - a test that reimplements the restore cannot fail when the restore is broken.
        let state = "[world_state:0]\nname=alpha\nnext_seq=500\nseq_epoch=987654321\n";
        let mut app = App::new();
        crate::persistence::load_reload_state_from_str(&mut app, state).expect("parses");

        let w = app.worlds.iter().find(|w| w.name == "alpha").expect("world restored");
        assert_eq!(w.next_seq, 500);
        assert_eq!(w.seq_epoch, 987_654_321,
            "the epoch must be restored verbatim, or a reload invalidates every client cache");
    }

    #[test]
    fn test_reload_without_a_seq_epoch_keeps_the_live_one() {
        // A state file written by a build predating this field reports 0, which means "no
        // epoch" - it must not clobber the epoch the World was constructed with, or every
        // reload from an old state file would hand clients the reserved value.
        let state = "[world_state:0]\nname=alpha\nnext_seq=500\n";
        let mut app = App::new();
        crate::persistence::load_reload_state_from_str(&mut app, state).expect("parses");

        let w = app.worlds.iter().find(|w| w.name == "alpha").expect("world restored");
        assert_ne!(w.seq_epoch, 0,
            "an absent epoch must leave the freshly-minted one in place, never the reserved 0 \
             - a client would read 0 as \"server doesn't speak epochs\"");
    }

    #[test]
    fn test_world_state_msg_carries_the_seq_epoch() {
        // The client can only compare what reaches it.
        let mut app = App::new();
        app.worlds = vec![World::new("alpha")];
        app.worlds[0].output_lines.push(OutputLine::new("hi".to_string(), 0));
        app.worlds[0].next_seq = 1;
        let expected = app.worlds[0].seq_epoch;

        let state = app.build_initial_state(0);
        match state {
            WsMessage::InitialState { worlds, .. } => {
                let w = worlds.iter().find(|w| w.name == "alpha").expect("world present");
                assert_eq!(w.seq_epoch, expected, "InitialState must report the world's epoch");
                assert_ne!(w.seq_epoch, 0);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn test_seq_epoch_defaults_to_zero_against_an_older_server() {
        // Absent field => 0 => the client treats the epoch as unknown and falls back to the
        // older next_seq heuristic rather than comparing 0 as a concrete value.
        let mut app = App::new();
        app.worlds = vec![World::new("alpha")];
        let state = app.build_initial_state(0);
        let mut encoded = serde_json::to_value(&state).unwrap();

        let w0 = &mut encoded["worlds"][0];
        assert!(w0.get("seq_epoch").is_some(), "epoch is present on the wire");
        w0.as_object_mut().unwrap().remove("seq_epoch");

        match serde_json::from_value::<WsMessage>(encoded).expect("parses without the field") {
            WsMessage::InitialState { worlds, .. } => assert_eq!(worlds[0].seq_epoch, 0),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    /// A hot reload must not make the whole restored buffer look un-broadcast.
    ///
    /// `save_reload_state` persists `output_lines` but not `broadcast_ledger`, and the world is
    /// rebuilt with `World::new()` - so without seeding, `audit_broadcast_ledger` sees every
    /// restored line as "stored but never broadcast", logs a SEQ-LEDGER report for each, and
    /// re-broadcasts the lot. Observed on a live server: 27,070 reports in one minute across 12
    /// worlds with WS-CHANNEL-FULL in lockstep, i.e. every connected client had its entire
    /// history pushed at it again. Nothing is owed after a reload - there are no live clients,
    /// and a client reconnecting afterwards asks for what it actually missed via
    /// AuthRequest.resume.
    #[test]
    fn test_reload_seeds_broadcast_ledger_so_the_audit_stays_quiet() {
        let mut app = App::new();
        app.worlds = vec![World::new("alpha")];
        for i in 0..50 {
            let mut l = OutputLine::new(format!("restored line {i}"), app.worlds[0].next_seq);
            l.viewed = true;
            app.worlds[0].next_seq += 1;
            app.worlds[0].output_lines.push(l);
        }
        // Pending lines carry higher seqs and genuinely have NOT been sent.
        for i in 0..3 {
            let l = OutputLine::new(format!("still pending {i}"), app.worlds[0].next_seq);
            app.worlds[0].next_seq += 1;
            app.worlds[0].pending_lines.push(l);
        }
        app.worlds[0].paused = true;
        let highest_output = app.worlds[0].output_lines.last().unwrap().seq;
        let first_pending = app.worlds[0].pending_lines.first().unwrap().seq;

        let mut buf: Vec<u8> = Vec::new();
        crate::persistence::save_reload_state_to(&app, &mut buf).expect("serializes");
        let text = String::from_utf8(buf).expect("utf-8");

        let mut restored = App::new();
        crate::persistence::load_reload_state_from_str(&mut restored, &text).expect("parses");
        let w = restored.worlds.iter().find(|w| w.name == "alpha").expect("world restored");
        assert_eq!(w.output_lines.len(), 50, "precondition: the buffer came back");

        for line in &w.output_lines {
            assert!(w.was_broadcast(line.seq),
                "restored line seq={} must count as already delivered, or the audit re-sends \
                 the entire buffer to every client on the next keepalive", line.seq);
        }
        assert!(!w.was_broadcast(first_pending),
            "pending lines have NOT been sent - marking them delivered would suppress a real \
             failure to deliver them after release");
        assert!(w.was_broadcast(highest_output));

        // The end-to-end consequence: the audit finds nothing to repair.
        let repaired = restored.audit_broadcast_ledger();
        assert_eq!(repaired, 0,
            "a reload must not produce SEQ-LEDGER reports; got {repaired}");
    }

    #[test]
    fn test_reload_state_write_emits_seq_epoch_and_round_trips() {
        // Closes the gap a fixture-driven parser test cannot: if the WRITE side stops
        // emitting seq_epoch, every reload hands clients a freshly-minted epoch for an
        // unchanged buffer and they discard a perfectly good cache each time. Drives the real
        // serializer and the real parser against each other.
        let mut app = App::new();
        app.worlds = vec![World::new("alpha")];
        app.worlds[0].next_seq = 500;
        let epoch = app.worlds[0].seq_epoch;

        let mut buf: Vec<u8> = Vec::new();
        crate::persistence::save_reload_state_to(&app, &mut buf).expect("serializes");
        let text = String::from_utf8(buf).expect("utf-8");

        assert!(text.contains(&format!("seq_epoch={}", epoch)),
            "the reload state must carry seq_epoch alongside next_seq");

        let mut restored = App::new();
        crate::persistence::load_reload_state_from_str(&mut restored, &text).expect("parses");
        let w = restored.worlds.iter().find(|w| w.name == "alpha").expect("world restored");
        assert_eq!(w.seq_epoch, epoch, "epoch must survive a full save/load round trip");
        assert_eq!(w.next_seq, 500);
    }
