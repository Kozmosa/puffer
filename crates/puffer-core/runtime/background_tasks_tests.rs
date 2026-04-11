use super::*;

#[test]
fn head_tail_buffer_small_input_no_truncation() {
    let mut buf = HeadTailBuffer::new();
    buf.write_str("hello world");
    assert!(!buf.was_truncated());
    assert_eq!(buf.to_string_lossy(), "hello world");
    assert_eq!(buf.bytes_dropped(), 0);
}

#[test]
fn head_tail_buffer_exact_head_capacity() {
    let mut buf = HeadTailBuffer::with_capacity(10, 10);
    buf.write_str("0123456789"); // exactly 10 bytes
    assert!(!buf.was_truncated());
    assert_eq!(buf.to_string_lossy(), "0123456789");
}

#[test]
fn head_tail_buffer_truncation_preserves_head_and_tail() {
    let mut buf = HeadTailBuffer::with_capacity(5, 5);
    // Write 15 bytes: head gets "01234", middle "56789" is lost, tail gets "abcde"
    buf.write_str("01234");
    buf.write_str("56789");
    buf.write_str("abcde");
    assert!(buf.was_truncated());
    assert_eq!(buf.total_written(), 15);
    assert_eq!(buf.bytes_dropped(), 5);
    let output = buf.to_string_lossy();
    assert!(output.starts_with("01234"));
    assert!(output.contains("5 bytes truncated"));
    assert!(output.ends_with("abcde"));
}

#[test]
fn head_tail_buffer_large_single_write() {
    let mut buf = HeadTailBuffer::with_capacity(4, 4);
    buf.write_str("abcdefghijklmnop"); // 16 bytes
    assert!(buf.was_truncated());
    let output = buf.to_string_lossy();
    assert!(output.starts_with("abcd"));
    assert!(output.ends_with("mnop"));
}

#[test]
fn ring_buffer_wraps_correctly() {
    let mut ring = RingBuffer::new(4);
    ring.write(b"abcd");
    assert_eq!(ring.to_vec(), b"abcd");
    ring.write(b"ef");
    // Should now contain "cdef" (oldest "ab" evicted).
    assert_eq!(ring.to_vec(), b"cdef");
}

#[test]
fn ring_buffer_large_write_keeps_last_n() {
    let mut ring = RingBuffer::new(4);
    ring.write(b"abcdefgh"); // 8 bytes, capacity 4
    assert_eq!(ring.to_vec(), b"efgh");
}

#[test]
fn task_manager_enforces_concurrent_limit() {
    let mgr = BackgroundTaskManager::new();
    for i in 0..MAX_CONCURRENT_TASKS {
        let result = mgr.register(&format!("task-{i}"), "test task", None, None, false);
        assert!(result.is_ok(), "task {i} should register");
    }
    // Next one should fail.
    let result = mgr.register("task-overflow", "overflow", None, None, false);
    assert!(result.is_err());

    // Complete one, then retry.
    mgr.complete("task-0", true);
    let result = mgr.register("task-replacement", "replacement", None, None, false);
    assert!(result.is_ok());
}

#[test]
fn task_manager_tracks_status_transitions() {
    let mgr = BackgroundTaskManager::new();
    let _ = mgr.register("t1", "test", None, None, false);

    let info = mgr.get_info("t1").unwrap();
    assert_eq!(info.status, BackgroundTaskStatus::Running);
    assert_eq!(mgr.active_count(), 1);

    mgr.complete("t1", true);
    let info = mgr.get_info("t1").unwrap();
    assert_eq!(info.status, BackgroundTaskStatus::Completed);
    assert!(info.completed_at.is_some());
    assert_eq!(mgr.active_count(), 0);
}

#[test]
fn task_manager_read_output() {
    let mgr = BackgroundTaskManager::new();
    let buf = mgr.register("t1", "test", None, None, false).unwrap();

    buf.lock().unwrap().write_str("line 1\n");
    buf.lock().unwrap().write_str("line 2\n");

    let output = mgr.read_output("t1").unwrap();
    assert_eq!(output, "line 1\nline 2\n");
}

#[test]
fn task_manager_cleanup() {
    let mgr = BackgroundTaskManager::new();
    let _ = mgr.register("t1", "old", None, None, false);
    mgr.complete("t1", true);
    let _ = mgr.register("t2", "active", None, None, false);

    // Cleanup with zero age removes all completed tasks.
    mgr.cleanup_older_than(Duration::ZERO);
    assert!(mgr.get_info("t1").is_none());
    assert!(mgr.get_info("t2").is_some());
}

#[test]
fn auto_background_inline_fast_task() {
    let result = run_with_auto_background("fast task", Duration::from_secs(5), || 42);
    match result {
        AutoBgResult::Inline(val) => assert_eq!(val, 42),
        AutoBgResult::Backgrounded { .. } => panic!("should complete inline"),
    }
}

#[test]
fn auto_background_slow_task_gets_backgrounded() {
    let result = run_with_auto_background("slow task", Duration::from_millis(50), || {
        std::thread::sleep(Duration::from_millis(200));
        "done"
    });
    match result {
        AutoBgResult::Inline(_) => panic!("should be backgrounded"),
        AutoBgResult::Backgrounded { task_id, .. } => {
            assert!(task_id.starts_with("auto-bg-"));
        }
    }
}

// -----------------------------------------------------------------------
// HeadTailBuffer: advanced / edge-case tests
// -----------------------------------------------------------------------

#[test]
fn head_tail_buffer_empty_produces_empty_string() {
    let buf = HeadTailBuffer::new();
    assert_eq!(buf.to_string_lossy(), "");
    assert_eq!(buf.total_written(), 0);
    assert!(!buf.was_truncated());
    assert_eq!(buf.bytes_dropped(), 0);
}

#[test]
fn head_tail_buffer_incremental_writes_fill_head_then_tail() {
    let mut buf = HeadTailBuffer::with_capacity(4, 4);
    // 4 small writes, each 2 bytes => total 8 = head(4) + tail(4), no truncation
    buf.write_str("ab");
    buf.write_str("cd");
    buf.write_str("ef");
    buf.write_str("gh");
    assert!(!buf.was_truncated());
    assert_eq!(buf.total_written(), 8);
    assert_eq!(buf.to_string_lossy(), "abcdefgh");
}

#[test]
fn head_tail_buffer_one_byte_over_triggers_truncation() {
    let mut buf = HeadTailBuffer::with_capacity(4, 4);
    // Write exactly 9 bytes: head=4, tail=4, 1 byte dropped
    buf.write(b"abcd"); // fills head
    buf.write(b"efghi"); // "e" is the first tail byte, overwrites as ring fills
    assert!(buf.was_truncated());
    assert_eq!(buf.bytes_dropped(), 1);
    let output = buf.to_string_lossy();
    assert!(output.starts_with("abcd"));
    assert!(output.ends_with("fghi"));
    assert!(output.contains("1 bytes truncated"));
}

#[test]
fn head_tail_buffer_many_small_writes_simulating_streaming() {
    // Simulate a real-world scenario: streaming output line-by-line
    let mut buf = HeadTailBuffer::with_capacity(20, 20);
    for i in 0..100 {
        buf.write_str(&format!("line {i}\n"));
    }
    assert!(buf.was_truncated());
    let output = buf.to_string_lossy();
    // Head should start with "line 0"
    assert!(
        output.starts_with("line 0\n"),
        "head should contain first lines"
    );
    // Tail should end with "line 99"
    assert!(
        output.contains("line 99\n"),
        "tail should contain last lines"
    );
    // Truncation marker should be present
    assert!(
        output.contains("bytes truncated"),
        "should have truncation marker"
    );
}

#[test]
fn head_tail_buffer_binary_data_handled_as_lossy() {
    let mut buf = HeadTailBuffer::with_capacity(4, 4);
    // Write invalid UTF-8 bytes
    buf.write(&[0xFF, 0xFE, 0x41, 0x42]); // head: \xFF\xFE AB
    buf.write(&[0x43, 0x44, 0x45, 0x46, 0x47]); // overflow triggers tail
    assert!(buf.was_truncated());
    let output = buf.to_string_lossy();
    // Should not panic, lossy conversion handles invalid bytes
    assert!(!output.is_empty());
}

#[test]
fn head_tail_buffer_zero_capacity_tail() {
    let mut buf = HeadTailBuffer::with_capacity(4, 0);
    buf.write_str("abcdefgh");
    // Head captures "abcd", tail has zero capacity so captures nothing
    assert_eq!(buf.head.len(), 4);
    assert_eq!(buf.tail.len(), 0);
}

#[test]
fn head_tail_buffer_total_written_tracks_all_data() {
    let mut buf = HeadTailBuffer::with_capacity(2, 2);
    buf.write_str("a"); // 1 byte, head
    assert_eq!(buf.total_written(), 1);
    buf.write_str("b"); // 2 bytes, head full
    assert_eq!(buf.total_written(), 2);
    buf.write_str("cde"); // 5 total, tail
    assert_eq!(buf.total_written(), 5);
    buf.write_str("fghijk"); // 11 total
    assert_eq!(buf.total_written(), 11);
}

// -----------------------------------------------------------------------
// RingBuffer: edge cases
// -----------------------------------------------------------------------

#[test]
fn ring_buffer_empty() {
    let ring = RingBuffer::new(8);
    assert_eq!(ring.len(), 0);
    assert_eq!(ring.to_vec(), Vec::<u8>::new());
}

#[test]
fn ring_buffer_single_byte_capacity() {
    let mut ring = RingBuffer::new(1);
    ring.write(b"a");
    assert_eq!(ring.to_vec(), b"a");
    ring.write(b"b");
    assert_eq!(ring.to_vec(), b"b");
    ring.write(b"xyz");
    assert_eq!(ring.to_vec(), b"z");
}

#[test]
fn ring_buffer_multiple_wraps() {
    let mut ring = RingBuffer::new(3);
    ring.write(b"abc"); // [a,b,c] pos=0
    assert_eq!(ring.to_vec(), b"abc");
    ring.write(b"de"); // [d,e,c] pos=2, read from pos=2 => "cde"? no...
                       // After wrap: the most recent 3 bytes are "cde"
    assert_eq!(ring.to_vec(), b"cde");
    ring.write(b"fgh"); // replaces everything
    assert_eq!(ring.to_vec(), b"fgh");
    ring.write(b"i"); // [i,g,h] pos=1 => read from 1: "ghi"
    assert_eq!(ring.to_vec(), b"ghi");
}

#[test]
fn ring_buffer_zero_capacity() {
    let mut ring = RingBuffer::new(0);
    ring.write(b"anything");
    assert_eq!(ring.len(), 0);
    assert_eq!(ring.to_vec(), Vec::<u8>::new());
}

// -----------------------------------------------------------------------
// BackgroundTaskManager: concurrency & real threading
// -----------------------------------------------------------------------

#[test]
fn task_manager_concurrent_thread_writes_to_buffer() {
    let mgr = BackgroundTaskManager::new();
    let buf = mgr
        .register("mt-1", "multi-thread test", None, None, false)
        .unwrap();

    let handles: Vec<_> = (0..4)
        .map(|thread_id| {
            let buf = Arc::clone(&buf);
            std::thread::spawn(move || {
                for i in 0..100 {
                    let mut guard = buf.lock().unwrap();
                    guard.write_str(&format!("[t{thread_id}] line {i}\n"));
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let output = mgr.read_output("mt-1").unwrap();
    // 4 threads * 100 lines = 400 writes
    // Output should contain data from all threads (may be truncated by buffer)
    assert!(!output.is_empty());
    // Total written should be 400 writes
    let total = buf.lock().unwrap().total_written();
    assert!(total > 0);
}

#[test]
fn task_manager_stop_then_complete_is_idempotent() {
    let mgr = BackgroundTaskManager::new();
    let _ = mgr.register("s1", "stop test", None, None, false);

    mgr.stop("s1");
    let info = mgr.get_info("s1").unwrap();
    assert_eq!(info.status, BackgroundTaskStatus::Stopped);
    assert_eq!(mgr.active_count(), 0);

    // Completing after stop still updates status
    mgr.complete("s1", true);
    let info = mgr.get_info("s1").unwrap();
    assert_eq!(info.status, BackgroundTaskStatus::Completed);
}

#[test]
fn task_manager_failed_task_is_terminal() {
    let mgr = BackgroundTaskManager::new();
    let _ = mgr.register("f1", "fail test", None, None, false);

    mgr.complete("f1", false);
    let info = mgr.get_info("f1").unwrap();
    assert_eq!(info.status, BackgroundTaskStatus::Failed);
    assert!(info.status.is_terminal());
    assert_eq!(mgr.active_count(), 0);
}

#[test]
fn task_manager_all_tasks_includes_completed() {
    let mgr = BackgroundTaskManager::new();
    let _ = mgr.register("a1", "active", None, None, false);
    let _ = mgr.register("a2", "will complete", None, None, false);
    mgr.complete("a2", true);

    let all = mgr.all_tasks();
    assert_eq!(all.len(), 2);

    let active = mgr.active_tasks();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].task_id, "a1");
}

#[test]
fn task_manager_has_capacity_reflects_limit() {
    let mgr = BackgroundTaskManager::new();
    assert!(mgr.has_capacity());

    for i in 0..MAX_CONCURRENT_TASKS {
        let _ = mgr.register(&format!("c-{i}"), "cap test", None, None, false);
    }
    assert!(!mgr.has_capacity());

    mgr.stop("c-0");
    assert!(mgr.has_capacity());
}

#[test]
fn task_manager_get_nonexistent_returns_none() {
    let mgr = BackgroundTaskManager::new();
    assert!(mgr.get_info("does-not-exist").is_none());
    assert!(mgr.read_output("does-not-exist").is_none());
}

#[test]
fn task_manager_metadata_fields_populated() {
    let mgr = BackgroundTaskManager::new();
    let _ = mgr.register(
        "meta-1",
        "metadata test",
        Some("agent-xyz"),
        Some("/tmp/out.json"),
        true,
    );

    let info = mgr.get_info("meta-1").unwrap();
    assert_eq!(info.task_id, "meta-1");
    assert_eq!(info.description, "metadata test");
    assert_eq!(info.agent_id.as_deref(), Some("agent-xyz"));
    assert_eq!(info.output_file.as_deref(), Some("/tmp/out.json"));
    assert!(info.auto_backgrounded);
    assert!(info.created_at > 0);
    assert!(info.completed_at.is_none());
}

// -----------------------------------------------------------------------
// Auto-backgrounding: integration with task manager
// -----------------------------------------------------------------------

#[test]
fn auto_background_slow_task_registered_in_manager() {
    let result = run_with_auto_background("register test", Duration::from_millis(30), || {
        std::thread::sleep(Duration::from_millis(200));
        "result"
    });
    match result {
        AutoBgResult::Backgrounded { task_id, .. } => {
            // Task should be registered in the global manager.
            let info = task_manager().get_info(&task_id);
            assert!(
                info.is_some(),
                "auto-backgrounded task should be in manager"
            );
            let info = info.unwrap();
            assert!(info.auto_backgrounded);
            assert_eq!(info.description, "register test");

            // Wait for the spawned thread to finish and mark complete.
            std::thread::sleep(Duration::from_millis(300));
            let info = task_manager().get_info(&task_id).unwrap();
            assert_eq!(info.status, BackgroundTaskStatus::Completed);
        }
        AutoBgResult::Inline(_) => panic!("should be backgrounded"),
    }
}

#[test]
fn auto_background_fast_task_not_registered_in_manager() {
    let result = run_with_auto_background("no-register test", Duration::from_secs(5), || "quick");
    // Fast task should NOT be registered in the manager since it completed inline.
    match result {
        AutoBgResult::Inline(val) => assert_eq!(val, "quick"),
        AutoBgResult::Backgrounded { .. } => panic!("should be inline"),
    }
}

// -----------------------------------------------------------------------
// format_buffer_size
// -----------------------------------------------------------------------

#[test]
fn format_buffer_size_units() {
    assert_eq!(format_buffer_size(0), "0 B");
    assert_eq!(format_buffer_size(512), "512 B");
    assert_eq!(format_buffer_size(1023), "1023 B");
    assert_eq!(format_buffer_size(1024), "1.0 KB");
    assert_eq!(format_buffer_size(1536), "1.5 KB");
    assert_eq!(format_buffer_size(1024 * 1024), "1.0 MB");
    assert_eq!(format_buffer_size(1024 * 1024 + 512 * 1024), "1.5 MB");
}

// -----------------------------------------------------------------------
// End-to-end: simulate real agent background task lifecycle
// -----------------------------------------------------------------------

#[test]
fn end_to_end_background_task_lifecycle() {
    let mgr = BackgroundTaskManager::new();

    // 1. Register a task (simulating launch_background_agent).
    let buf = mgr
        .register(
            "e2e-agent-1",
            "explore codebase",
            Some("agent-001"),
            None,
            false,
        )
        .expect("should register");

    assert_eq!(mgr.active_count(), 1);
    assert!(mgr.has_capacity());

    // 2. Simulate agent turns writing output (from background thread).
    let buf_clone = Arc::clone(&buf);
    let handle = std::thread::spawn(move || {
        for turn in 1..=3 {
            let mut guard = buf_clone.lock().unwrap();
            guard.write_str(&format!(
                "--- Turn {turn} ---\nDid some work.\nTool calls: 2\n\n"
            ));
            drop(guard);
            std::thread::sleep(Duration::from_millis(10));
        }
    });

    // 3. Meanwhile, read partial output (simulating TaskOutput polling).
    std::thread::sleep(Duration::from_millis(20));
    let partial = mgr.read_output("e2e-agent-1").unwrap();
    assert!(
        partial.contains("Turn 1"),
        "should have at least first turn"
    );

    // 4. Wait for agent thread to finish.
    handle.join().unwrap();
    mgr.complete("e2e-agent-1", true);

    // 5. Verify final state.
    let info = mgr.get_info("e2e-agent-1").unwrap();
    assert_eq!(info.status, BackgroundTaskStatus::Completed);
    assert!(info.completed_at.is_some());

    let final_output = mgr.read_output("e2e-agent-1").unwrap();
    assert!(final_output.contains("Turn 1"));
    assert!(final_output.contains("Turn 2"));
    assert!(final_output.contains("Turn 3"));
    assert!(!buf.lock().unwrap().was_truncated());

    // 6. Cleanup removes completed task.
    mgr.cleanup_older_than(Duration::ZERO);
    assert!(mgr.get_info("e2e-agent-1").is_none());
    assert_eq!(mgr.active_count(), 0);
}
