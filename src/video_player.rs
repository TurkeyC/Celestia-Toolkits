use crate::config::Config;
use crate::ffmpeg::*;
use crate::kitty;
use std::ffi::{CString, c_int};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// CPAL and Ringbuf imports
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::HeapRb;
use ringbuf::traits::{Producer, Consumer, Split};

enum VideoMessage {
    Frame { rgba: Vec<u8>, pts: f64 },
    LoopReset,
}

pub fn play(config: &Config, file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Set up Ctrl-C handler for graceful exit and cleanup
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    unsafe {
        av_log_set_level(AV_LOG_QUIET);
        let mut format_ctx: *mut AVFormatContext = std::ptr::null_mut();
        let c_file_path = CString::new(file_path)?;

        let ret = avformat_open_input(
            &mut format_ctx,
            c_file_path.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if ret != 0 {
            return Err(format!("Could not open video file: {}", file_path).into());
        }

        let ret = avformat_find_stream_info(format_ctx, std::ptr::null_mut());
        if ret < 0 {
            avformat_close_input(&mut format_ctx);
            return Err("Could not find stream information".into());
        }

        // Find the first video and audio streams
        let mut video_stream_index = -1;
        let mut audio_stream_index = -1;
        let mut video_codecpar_ptr: *mut AVCodecParameters = std::ptr::null_mut();
        let mut audio_codecpar_ptr: *mut AVCodecParameters = std::ptr::null_mut();
        let mut video_stream_ptr: *mut AVStream = std::ptr::null_mut();

        for i in 0..(*format_ctx).nb_streams {
            let stream_ptr_ptr = (*format_ctx).streams.add(i as usize);
            if stream_ptr_ptr.is_null() {
                continue;
            }
            let stream = *stream_ptr_ptr;
            if stream.is_null() {
                continue;
            }
            let codecpar = (*stream).codecpar;
            if codecpar.is_null() {
                continue;
            }
            if (*codecpar).codec_type == AVMEDIA_TYPE_VIDEO && video_stream_index == -1 {
                video_stream_index = i as i32;
                video_codecpar_ptr = codecpar;
                video_stream_ptr = stream;
            } else if (*codecpar).codec_type == AVMEDIA_TYPE_AUDIO && audio_stream_index == -1 {
                audio_stream_index = i as i32;
                audio_codecpar_ptr = codecpar;
            }
        }

        if video_stream_index == -1 {
            avformat_close_input(&mut format_ctx);
            return Err("Could not find a video stream".into());
        }

        // Find and open video decoder
        let video_decoder = avcodec_find_decoder((*video_codecpar_ptr).codec_id);
        if video_decoder.is_null() {
            avformat_close_input(&mut format_ctx);
            return Err("Video decoder not found".into());
        }

        let codec_ctx = avcodec_alloc_context3(video_decoder);
        if codec_ctx.is_null() {
            avformat_close_input(&mut format_ctx);
            return Err("Could not allocate video codec context".into());
        }

        let ret = avcodec_parameters_to_context(codec_ctx, video_codecpar_ptr);
        if ret < 0 {
            avcodec_free_context(&mut { codec_ctx });
            avformat_close_input(&mut format_ctx);
            return Err("Could not copy video codec parameters to context".into());
        }

        let ret = avcodec_open2(codec_ctx, video_decoder, std::ptr::null_mut());
        if ret < 0 {
            avcodec_free_context(&mut { codec_ctx });
            avformat_close_input(&mut format_ctx);
            return Err("Could not open video codec".into());
        }

        // Setup Audio if available
        let mut audio_codec_ctx: *mut AVCodecContext = std::ptr::null_mut();
        let mut has_audio = audio_stream_index != -1;
        if has_audio {
            let audio_decoder = avcodec_find_decoder((*audio_codecpar_ptr).codec_id);
            if audio_decoder.is_null() {
                has_audio = false;
            } else {
                audio_codec_ctx = avcodec_alloc_context3(audio_decoder);
                if audio_codec_ctx.is_null() {
                    has_audio = false;
                } else {
                    let ret = avcodec_parameters_to_context(audio_codec_ctx, audio_codecpar_ptr);
                    if ret < 0 {
                        avcodec_free_context(&mut audio_codec_ctx);
                        audio_codec_ctx = std::ptr::null_mut();
                        has_audio = false;
                    } else {
                        let ret = avcodec_open2(audio_codec_ctx, audio_decoder, std::ptr::null_mut());
                        if ret < 0 {
                            avcodec_free_context(&mut audio_codec_ctx);
                            audio_codec_ctx = std::ptr::null_mut();
                            has_audio = false;
                        }
                    }
                }
            }
        }

        // Initialize CPAL audio output
        let mut _cpal_stream: Option<cpal::Stream> = None;
        let mut target_channels = 2;
        let mut target_sample_rate = 48000;
        let audio_clock = Arc::new(AtomicU64::new(0));
        let reset_audio = Arc::new(AtomicBool::new(false));

        let mut swr_ctx: *mut SwrContext = std::ptr::null_mut();
        let mut audio_producer = None;

        if has_audio {
            let host = cpal::default_host();
            if let Some(device) = host.default_output_device() {
                if let Ok(config_supported) = device.default_output_config() {
                    let audio_config: cpal::StreamConfig = config_supported.into();
                    target_channels = audio_config.channels;
                    target_sample_rate = audio_config.sample_rate;

                    // Heap-allocated ring buffer
                    let rb = HeapRb::<f32>::new(target_sample_rate as usize * 2);
                    let (prod, mut cons) = rb.split();
                    audio_producer = Some(prod);

                    let audio_clock_cb = audio_clock.clone();
                    let reset_audio_cb = reset_audio.clone();

                    let stream_res = device.build_output_stream(
                        &audio_config,
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            if reset_audio_cb.load(Ordering::SeqCst) {
                                while cons.try_pop().is_some() {}
                                reset_audio_cb.store(false, Ordering::SeqCst);
                            }
                            let mut read = 0;
                            for sample in data.iter_mut() {
                                if let Some(val) = cons.try_pop() {
                                    *sample = val;
                                    read += 1;
                                } else {
                                    *sample = 0.0;
                                }
                            }
                            audio_clock_cb.fetch_add(read as u64, Ordering::SeqCst);
                        },
                        move |err| {
                            eprintln!("CPAL Audio stream error: {}", err);
                        },
                        None,
                    );

                    if let Ok(stream) = stream_res {
                        if stream.play().is_ok() {
                            _cpal_stream = Some(stream);

                            // Configure SwrContext
                            let mut in_ch_layout = AVChannelLayout::default();
                            if (*audio_codecpar_ptr).ch_layout.nb_channels > 0 {
                                av_channel_layout_copy(&mut in_ch_layout, &(*audio_codecpar_ptr).ch_layout);
                            } else {
                                av_channel_layout_default(&mut in_ch_layout, 2);
                            }

                            let mut out_ch_layout = AVChannelLayout::default();
                            av_channel_layout_default(&mut out_ch_layout, target_channels as c_int);

                            let ret = swr_alloc_set_opts2(
                                &mut swr_ctx,
                                &out_ch_layout,
                                AV_SAMPLE_FMT_FLT,
                                target_sample_rate as c_int,
                                &in_ch_layout,
                                (*audio_codecpar_ptr).format,
                                (*audio_codecpar_ptr).sample_rate,
                                0,
                                std::ptr::null_mut(),
                            );

                            av_channel_layout_uninit(&mut in_ch_layout);
                            av_channel_layout_uninit(&mut out_ch_layout);

                            if ret >= 0 && !swr_ctx.is_null() {
                                let init_ret = swr_init(swr_ctx);
                                if init_ret < 0 {
                                    swr_free(&mut swr_ctx);
                                    _cpal_stream = None;
                                    has_audio = false;
                                }
                            } else {
                                if !swr_ctx.is_null() {
                                    swr_free(&mut swr_ctx);
                                }
                                _cpal_stream = None;
                                has_audio = false;
                            }
                        } else {
                            has_audio = false;
                        }
                    } else {
                        has_audio = false;
                    }
                } else {
                    has_audio = false;
                }
            } else {
                has_audio = false;
            }
        }

        // Calculate size and aspect ratio for Video
        let (term_cols, _term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let target_cols = ((term_cols as f32) * config.zoom) as u32;
        let target_cols = std::cmp::max(1, target_cols);

        let orig_w = (*video_codecpar_ptr).width;
        let orig_h = (*video_codecpar_ptr).height;

        let target_rows = (((target_cols as f32 * orig_h as f32) / orig_w as f32) * 0.5) as u32;
        let target_rows = std::cmp::max(1, target_rows);

        let target_w_px = target_cols * 10;
        let target_h_px = target_rows * 20;

        // Initialize SwsContext
        let src_format = (*video_codecpar_ptr).format;
        let sws_ctx = sws_getContext(
            orig_w,
            orig_h,
            src_format,
            target_w_px as i32,
            target_h_px as i32,
            AV_PIX_FMT_RGBA,
            SWS_BILINEAR,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );

        if sws_ctx.is_null() {
            if has_audio {
                swr_free(&mut swr_ctx);
                avcodec_free_context(&mut audio_codec_ctx);
            }
            avcodec_free_context(&mut { codec_ctx });
            avformat_close_input(&mut format_ctx);
            return Err("Could not initialize software scaler".into());
        }

        // Determine frame delay (timing)
        let fps_rational = (*video_stream_ptr).avg_frame_rate;
        let delay_secs = if fps_rational.num > 0 && fps_rational.den > 0 {
            fps_rational.den as f32 / fps_rational.num as f32
        } else {
            1.0 / 30.0
        };
        let frame_delay = config
            .fps
            .map(|f| Duration::from_secs_f32(1.0 / f))
            .unwrap_or_else(|| Duration::from_secs_f32(delay_secs));

        let pkt = av_packet_alloc();
        let frame = av_frame_alloc();
        let audio_frame = if has_audio { av_frame_alloc() } else { std::ptr::null_mut() };

        let mut output_buffer = vec![0u8; (target_w_px * target_h_px * 4) as usize];
        let dst_data: [*mut u8; 8] = [
            output_buffer.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ];
        let dst_linesize: [c_int; 8] = [
            (target_w_px * 4) as c_int,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];

        // Pre-allocate terminal rows to prevent scrolling drift during playback
        for _ in 0..target_rows {
            println!();
        }
        kitty::move_up_robust(target_rows as u16)?;

        // Set up the thread-safe channel for transferring decoded video frames.
        // When audio is present, we use a much larger capacity (1024 frames) to prevent the main
        // decoding/demuxing thread from blocking on sending video frames. If the main thread blocks,
        // it cannot read packets from the demuxer or decode audio, starving the audio ring buffer
        // and causing audio pauses/skips. The audio buffer itself (2 seconds of audio) naturally
        // throttles the demuxer to real-time. If there is no audio, we use a small capacity (16 frames)
        // to throttle the demuxer to the rendering thread's consumption speed.
        let video_channel_capacity = if has_audio { 1024 } else { 16 };
        let (video_sender, video_receiver) = std::sync::mpsc::sync_channel::<VideoMessage>(video_channel_capacity);

        // Spawn video rendering thread
        let running_render = running.clone();
        let audio_clock_render = audio_clock.clone();
        let has_audio_render = has_audio;
        let target_channels_render = target_channels;
        let target_sample_rate_render = target_sample_rate;
        let frame_delay_render = frame_delay;
        let target_w_px_render = target_w_px;
        let target_h_px_render = target_h_px;
        let target_cols_render = target_cols;
        let target_rows_render = target_rows;

        let render_thread = std::thread::spawn(move || {
            let mut first_video_pts: Option<f64> = None;
            let mut last_frame_time = Instant::now();

            while let Ok(msg) = video_receiver.recv() {
                match msg {
                    VideoMessage::Frame { rgba, pts } => {
                        if !running_render.load(Ordering::SeqCst) {
                            break;
                        }

                        if first_video_pts.is_none() {
                            first_video_pts = Some(pts);
                        }

                        let relative_video_time = pts - first_video_pts.unwrap();

                        if has_audio_render {
                            let played_samples = audio_clock_render.load(Ordering::SeqCst);
                            let audio_time_played = played_samples as f64 / (target_channels_render as f64 * target_sample_rate_render as f64);
                            let diff = relative_video_time - audio_time_played;

                            if diff > 0.010 {
                                // Video is too fast (early), sleep to align
                                let sleep_ms = (diff * 1000.0) as u64;
                                let chunk_size = 10;
                                let mut slept = 0;
                                while slept < sleep_ms && running_render.load(Ordering::SeqCst) {
                                    let to_sleep = std::cmp::min(chunk_size, sleep_ms - slept);
                                    std::thread::sleep(Duration::from_millis(to_sleep));
                                    slept += to_sleep;
                                }
                            } else if diff < -0.010 {
                                // Video is too slow (late), skip rendering
                                continue;
                            }
                        } else {
                            // Fallback to timer-based precise sleep
                            let elapsed = last_frame_time.elapsed();
                            if elapsed < frame_delay_render {
                                std::thread::sleep(frame_delay_render - elapsed);
                            }
                            last_frame_time = Instant::now();
                        }

                        // Draw using the Kitty protocol with prevent_cursor_move = true
                        let _ = kitty::write_rgba_frame(
                            &rgba,
                            target_w_px_render,
                            target_h_px_render,
                            target_cols_render,
                            target_rows_render,
                            true,
                        );
                    }
                    VideoMessage::LoopReset => {
                        first_video_pts = None;
                        last_frame_time = Instant::now();
                    }
                }
            }
        });

        'outer: loop {
            while running.load(Ordering::SeqCst) {
                let ret = av_read_frame(format_ctx, pkt);
                if ret != 0 {
                    // End of stream
                    break;
                }

                if (*pkt).stream_index == video_stream_index {
                    let ret = avcodec_send_packet(codec_ctx, pkt);
                    if ret == 0 {
                        while avcodec_receive_frame(codec_ctx, frame) == 0 {
                            if !running.load(Ordering::SeqCst) {
                                break;
                            }

                            // Calculate Video PTS
                            let tb = (*video_stream_ptr).time_base;
                            let frame_pts = (*frame).pts;
                            let video_pts = if tb.den > 0 {
                                frame_pts as f64 * (tb.num as f64 / tb.den as f64)
                            } else {
                                0.0
                            };

                            // Convert/scale frame to RGBA output_buffer
                            sws_scale(
                                sws_ctx,
                                (*frame).data.as_ptr() as *const *const u8,
                                (*frame).linesize.as_ptr(),
                                0,
                                orig_h,
                                dst_data.as_ptr() as *const *mut u8,
                                dst_linesize.as_ptr(),
                            );

                            // Send frame to rendering thread
                            let _ = video_sender.send(VideoMessage::Frame {
                                rgba: output_buffer.clone(),
                                pts: video_pts,
                            });
                        }
                    }
                } else if (*pkt).stream_index == audio_stream_index && has_audio {
                    let ret = avcodec_send_packet(audio_codec_ctx, pkt);
                    if ret == 0 {
                        while avcodec_receive_frame(audio_codec_ctx, audio_frame) == 0 {
                            if !running.load(Ordering::SeqCst) {
                                break;
                            }

                            // Calculate resampled output count
                            let max_out_samples = ((*audio_frame).nb_samples as i64 * target_sample_rate as i64 / (*audio_frame).sample_rate as i64 + 256) as c_int;
                            let mut resampled_buffer = vec![0.0f32; (max_out_samples * target_channels as c_int) as usize];

                            let mut out_ptrs: [*mut u8; 8] = [std::ptr::null_mut(); 8];
                            out_ptrs[0] = resampled_buffer.as_mut_ptr() as *mut u8;

                            let in_ptrs = (*audio_frame).extended_data as *const *const u8;

                            let converted = swr_convert(
                                swr_ctx,
                                out_ptrs.as_mut_ptr(),
                                max_out_samples,
                                in_ptrs,
                                (*audio_frame).nb_samples,
                            );

                            if converted > 0 {
                                if let Some(ref mut prod) = audio_producer {
                                    let sample_count = (converted * target_channels as c_int) as usize;
                                    for i in 0..sample_count {
                                        let sample = resampled_buffer[i];
                                        while running.load(Ordering::SeqCst) {
                                            match prod.try_push(sample) {
                                                Ok(_) => break,
                                                Err(_) => {
                                                    std::thread::sleep(Duration::from_micros(500));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                av_packet_unref(pkt);
            }

            if config.loop_video && running.load(Ordering::SeqCst) {
                // Seek back to start
                av_seek_frame(format_ctx, video_stream_index, 0, 4); // 4 = AVSEEK_FLAG_ANY
                avcodec_flush_buffers(codec_ctx);
                if has_audio {
                    avcodec_flush_buffers(audio_codec_ctx);
                    reset_audio.store(true, Ordering::SeqCst);
                    audio_clock.store(0, Ordering::SeqCst);
                }
                let _ = video_sender.send(VideoMessage::LoopReset);
            } else {
                break 'outer;
            }
        }

        // Drop the sender to let rendering thread know we are done
        drop(video_sender);

        // Wait for rendering thread to complete
        let _ = render_thread.join();

        // Clean up FFI allocations
        av_packet_free(&mut { pkt });
        av_frame_free(&mut { frame });
        if !audio_frame.is_null() {
            av_frame_free(&mut { audio_frame });
        }
        sws_freeContext(sws_ctx);
        if !swr_ctx.is_null() {
            swr_free(&mut swr_ctx);
        }
        avcodec_free_context(&mut { codec_ctx });
        if !audio_codec_ctx.is_null() {
            avcodec_free_context(&mut { audio_codec_ctx });
        }
        avformat_close_input(&mut format_ctx);

        // Move cursor to bottom-left after video ends
        let mut stdout = std::io::stdout().lock();
        let mut buf = Vec::new();
        let _ = crossterm::queue!(
            buf,
            crossterm::cursor::MoveDown(target_rows as u16),
            crossterm::cursor::MoveToColumn(0)
        );
        let _ = kitty::write_all_robust(&mut stdout, &buf);
        let _ = kitty::flush_robust(&mut stdout);
        println!();
    }
    Ok(())
}

