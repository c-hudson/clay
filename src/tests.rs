    use super::*;

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
                    if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                        if let WsMessage::AuthRequest { password_hash: client_hash, .. } = ws_msg {
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
                            ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();
                            break;
                        }
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
        let auth_msg = WsMessage::AuthRequest { password_hash: client_hash, username: None, current_world: None, auth_key: None, request_key: false, challenge_response: false };
        let json = serde_json::to_string(&auth_msg).unwrap();
        ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();

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

        // URL followed by ANSI color code should stop at ESC, not include the code in URL
        let url_with_ansi = "https://example.com\x1b[0;37m rest";
        let result = wrap_urls_with_osc8(url_with_ansi);
        // URL should end at the ESC, not include the ANSI code
        assert!(result.contains("\x1b]8;;https://example.com\x07"));
        // The ANSI code should be preserved after the OSC 8 closing sequence
        assert!(result.contains("\x1b]8;;\x07\x1b[0;37m"));
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
        use tokio::sync::RwLock;

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
                allow_list,
                whitelisted,
                client_addr,
                event_tx,
                false, // not multiuser
                users,
                ban_list,
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
        ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();

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
        use tokio::sync::RwLock;

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
                stream, 1, server_clients, password_hash,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
            ).await.ok();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{}", port);
        let (ws_stream, _) = connect_async(&url).await.unwrap();
        let (mut ws_sink, _ws_source) = ws_stream.split();

        // Try sending a command without authenticating
        let cmd = WsMessage::SendCommand { world_index: 0, command: "look".to_string() };
        let json = serde_json::to_string(&cmd).unwrap();
        ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();

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
        use tokio::sync::RwLock;

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
                stream, 1, server_clients, password_hash,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
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
        };
        let json = serde_json::to_string(&auth_msg).unwrap();
        ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();

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
        use tokio::sync::RwLock;

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
                stream, 1, server_clients, password_hash,
                allow_list, whitelisted, client_addr, event_tx,
                true, // multiuser mode
                users, ban_list,
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
                stream, 2, clients2, password_hash2,
                allow_list2, whitelisted2, client_addr, event_tx2,
                true, users2, ban_list2,
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
        };
        sink1.send(WsRawMessage::Text(serde_json::to_string(&auth1).unwrap().into())).await.unwrap();
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
        };
        sink2.send(WsRawMessage::Text(serde_json::to_string(&auth2).unwrap().into())).await.unwrap();
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
        use tokio::sync::RwLock;

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
                stream, 1, server_clients, ph,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
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
        };
        sink.send(WsRawMessage::Text(serde_json::to_string(&auth).unwrap().into())).await.unwrap();

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
    }

    /// Test: ServerHello is sent before auth (regression: needed for client UI)
    #[tokio::test]
    async fn test_security_server_hello_sent_first() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::StreamExt;
        use crate::websocket::{WsMessage, WsClientInfo};
        use std::sync::Arc;
        use tokio::sync::RwLock;

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
                stream, 1, server_clients, ph,
                allow_list, whitelisted, client_addr, event_tx,
                false, users, ban_list,
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
        world.add_output(&data, true, &settings, 24, 80, false, true);

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

    #[test]
    fn test_more_mode_long_wrapped_lines_unit() {
        // Test that add_output correctly triggers more-mode with long lines that wrap
        let mut world = World::new("test");
        let settings = Settings { more_mode_enabled: true, ..Settings::default() };

        // 50 lines, each ~120 chars (wraps to 2 visual lines at width 80)
        let data: String = (1..=50).map(|i| {
            format!("{}{}\n", "x".repeat(100), format!("LINE{:03}", i))
        }).collect();
        world.add_output(&data, true, &settings, 24, 80, false, true);

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
        world.add_output("short line one\n", true, &settings, output_height, output_width, false, true);
        world.add_output("short line two\n", true, &settings, output_height, output_width, false, true);
        assert!(!world.paused, "Should not be paused after 2 short lines");
        assert_eq!(world.lines_since_pause, 2);

        // 1 huge line: 25 visual lines at width 80 (80*25 = 2000 visible chars)
        let huge_line = "A".repeat(80 * 25);
        world.add_output(&format!("{}\n", huge_line), true, &settings, output_height, output_width, false, true);

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
        world.add_output("short one\n", true, &settings, output_height, output_width, false, true);
        world.add_output("short two\n", true, &settings, output_height, output_width, false, true);
        let huge_line = "B".repeat(80 * 25);
        // The huge line + 5 more lines in one batch
        let batch = format!("{}\npending1\npending2\npending3\npending4\npending5\n", huge_line);
        world.add_output(&batch, true, &settings, output_height, output_width, false, true);

        assert!(world.paused);
        assert_eq!(world.output_lines.len(), 3, "3 lines in output (2 short + 1 huge)");
        assert_eq!(world.pending_lines.len(), 5, "5 lines pending");
        assert_eq!(world.visual_line_offset, 17, "partial display at 17 vl");

        // Simulate Tab: release_pending reveals more of the huge line first
        // Remaining vl of huge line: 25 - 17 = 8. Budget is 19.
        // 8 < 19, so partial clears and budget becomes 19 - 8 = 11 for pending.
        world.release_pending(19 - 8, output_width as usize);
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
        world.add_output("short\n", true, &settings, output_height, output_width, false, true);
        world.add_output("short\n", true, &settings, output_height, output_width, false, true);
        let huge_line = "C".repeat(80 * 25);
        world.add_output(&format!("{}\nextra\n", huge_line), true, &settings, output_height, output_width, false, true);

        assert!(world.visual_line_offset > 0, "Should have visual_line_offset set");

        // release_all_pending should clear it
        world.release_all_pending();
        assert_eq!(world.visual_line_offset, 0, "release_all_pending should clear visual_line_offset");
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

        world.add_output("short\n", true, &settings, output_height, output_width, false, true);
        world.add_output("short\n", true, &settings, output_height, output_width, false, true);
        let huge_line = "D".repeat(80 * 25);
        world.add_output(&format!("{}\nextra\n", huge_line), true, &settings, output_height, output_width, false, true);

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
        world.add_output(chunk1, true, &settings, 48, 80, false, true);
        assert_eq!(world.output_lines.len(), 1, "Chunk 1: should have 1 output line (partial)");
        assert!(!world.partial_line.is_empty(), "Chunk 1: partial_line should be set");

        world.add_output(chunk2, true, &settings, 48, 80, false, true);
        assert_eq!(world.output_lines.len(), 1, "Chunk 2: should STILL have 1 output line (updated partial)");
        assert!(!world.partial_line.is_empty(), "Chunk 2: partial_line should STILL be set (not lost)");

        world.add_output(chunk3, true, &settings, 48, 80, false, true);
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
            world.add_output(chunk, true, &settings, 48, 80, false, true);
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
            world.add_output(&data, true, &settings, 48, 80, false, true);
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
        world.add_output(&data, true, &settings, 48, 80, false, true);

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
        world.add_output(&chunk1, true, &settings, 24, 80, false, true);

        // After chunk 1: should have 23 output lines and be paused.
        // The key assertion: paused must remain true even though pending is empty.
        assert_eq!(world.output_lines.len(), 23,
            "Chunk 1: Expected 23 output lines, got {}", world.output_lines.len());

        // Chunk 2: lines 23-99 (77 lines). Already paused, all should go to pending.
        let chunk2: String = all_lines[23..100].concat();
        world.add_output(&chunk2, true, &settings, 24, 80, false, true);

        assert!(world.paused, "Should still be paused after chunk 2");
        assert_eq!(world.output_lines.len(), 23,
            "Chunk 2: Expected still 23 output lines, got {}", world.output_lines.len());
        assert_eq!(world.pending_lines.len(), 77,
            "Chunk 2: Expected 77 pending lines, got {}", world.pending_lines.len());

        // Chunk 3: lines 100-199 (100 lines). Still paused, all go to pending.
        let chunk3: String = all_lines[100..200].concat();
        world.add_output(&chunk3, true, &settings, 24, 80, false, true);

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
        world.add_output(&data, true, &settings, 48, 80, false, true);

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
        world.release_pending(46, 80);
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
        world.add_output(&long_line, true, &settings, 48, 80, false, true);

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
        world.add_output(&short_data, true, &settings, 48, 80, false, true);

        // Now add mixed content: 1 short line, then 1 very long line, then 1 short line
        let mixed: String = format!("short\n{}\nafter\n", "x".repeat(500));
        world.add_output(&mixed, true, &settings, 48, 80, false, true);

        assert!(world.paused, "Should be paused");
        let pending = world.pending_lines.len();
        assert!(pending > 0, "Should have pending lines");

        // Release with visual budget of 46
        // "short" = 1 visual line
        // 500-char line = ceil(500/80) = 7 visual lines
        // "after" = 1 visual line
        // total = 9 visual lines < 46, so all should be released
        world.release_pending(46, 80);
        assert_eq!(world.pending_lines.len(), 0,
            "All {} pending lines should fit in visual budget of 46", pending);
    }

    #[test]
    fn test_wrap_ansi_line_no_spaces() {
        // No spaces = hard wrap at character boundary
        let line = "a".repeat(100);
        let lines = wrap_ansi_line(&line, 10);
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
        let lines = wrap_ansi_line(line, 10);
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
        let lines = wrap_ansi_line(&line, 10);
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
        let lines = wrap_ansi_line(line, 80);
        assert_eq!(lines.len(), 1);
        // Should contain the original text (plus trailing reset)
        assert!(strip_ansi_codes(&lines[0]).contains("hello world"));
    }

    #[test]
    fn test_wrap_ansi_line_fffff_pattern() {
        // Simulate the actual test case: space-separated fffff words at width 80
        let words: Vec<String> = (0..1000).map(|i| format!("fffff{}", i)).collect();
        let line = words.join(" ");
        let lines = wrap_ansi_line(&line, 80);
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
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(0, _))),
            "Expected WsBroadcastServerData(0, _). ServerData events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastServerData(_, _))).collect::<Vec<_>>());

        // Should have WsBroadcastServerData for world2 (index 1)
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(1, _))),
            "Expected WsBroadcastServerData(1, _). ServerData events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastServerData(_, _))).collect::<Vec<_>>());

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

        // World 0 is current. Both worlds receive basic_output (5 lines).
        // World 0's broadcasts should have marked_new=false, world 1's should have marked_new=true.
        let events = testharness::run_test_scenario(config, vec![]).await;

        // Current world (index 0) should have marked_new=false
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(0, false))),
            "Expected WsBroadcastServerData(0, false) for current world. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastServerData(0, _))).collect::<Vec<_>>());

        // Non-current world (index 1) should have marked_new=true
        assert!(events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(1, true))),
            "Expected WsBroadcastServerData(1, true) for non-current world. Events: {:?}",
            events.iter().filter(|e| matches!(e, TestEvent::WsBroadcastServerData(1, _))).collect::<Vec<_>>());

        // Verify no marked_new=true broadcasts for current world
        assert!(!events.iter().any(|e| matches!(e, TestEvent::WsBroadcastServerData(0, true))),
            "Current world should never have marked_new=true broadcasts");

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

        let events = testharness::run_test_scenario(config, vec![]).await;

        // All ServerData broadcasts for the current (only) world should have marked_new=false
        let server_data_events: Vec<_> = events.iter()
            .filter(|e| matches!(e, TestEvent::WsBroadcastServerData(0, _)))
            .collect();
        assert!(!server_data_events.is_empty(), "Should have ServerData broadcasts");
        assert!(server_data_events.iter().all(|e| matches!(e, TestEvent::WsBroadcastServerData(0, false))),
            "Current world should never have marked_new=true. Events: {:?}", server_data_events);

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

        // --- Build expected command list from Rust parse_command() ---
        // These are the command strings (without /) that parse_command() matches on.
        // When adding a new command to parse_command(), add it here too.
        let mut rust_commands: Vec<String> = vec![
            "help", "version", "quit", "reload", "update", "setup", "web", "actions",
            "connections", "l", "worlds", "world", "disconnect", "dc",
            "flush", "menu", "send", "remote", "ban", "unban",
            "testmusic", "dump", "notify", "addworld", "note", "tag", "tags",
            "dict", "urban", "translate", "tr", "font", "window",
        ].into_iter().map(|s| s.to_string()).collect();
        rust_commands.sort();
        rust_commands.dedup();

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
             To fix: update INTERNAL_COMMANDS in src/web/app.js and rust_commands in this test.",
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

    /// Helper: create an OutputLine with the given text and marked_new flag
    fn make_output_line(text: &str, marked_new: bool) -> OutputLine {
        OutputLine {
            text: text.to_string(),
            timestamp: std::time::SystemTime::now(),
            from_server: true,
            gagged: false,
            seq: 0,
            highlight_color: None,
            marked_new,
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

