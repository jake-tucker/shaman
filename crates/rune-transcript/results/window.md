### window, bridge Batch, rss at rest 99 MB
- open: asked→first frame encoded 102.3 ms, →on the display 122.4 ms; rss 122 MB
- 1068 rows (28 Messages, 1040 Activities) · asked→snapshot decoded 81.2 ms · crossed to UI 0.0 ms · indexed 0.2 ms
- 60 keystrokes into the composer: 60 frames; update ms p50 0.39 p95 0.46 max 0.49; encode ms p50 0.09 p95 0.12 max 0.20; 0 crossings carrying 0 updates
- stream 400 words at 80/s, at the foot: 418 frames; update ms p50 0.61 p95 0.80 max 1.07; encode ms p50 0.10 p95 0.15 max 123.23; 427 crossings carrying 430 updates; over 6.2 s
- stream 1200 words as fast as the Server sends, at the foot: 433 frames; update ms p50 0.79 p95 1.11 max 1.41; encode ms p50 0.10 p95 0.15 max 0.23; 1229 crossings carrying 1233 updates; over 3.9 s
- stream 400 words at 80/s, scrolled back four pages: 428 frames; update ms p50 0.42 p95 0.46 max 1.04; encode ms p50 0.10 p95 0.12 max 0.18; 427 crossings carrying 429 updates; over 6.2 s
- rss at the end 130 MB
### window, bridge Update, rss at rest 99 MB
- open: asked→first frame encoded 91.2 ms, →on the display 115.1 ms; rss 125 MB
- 1068 rows (28 Messages, 1040 Activities) · asked→snapshot decoded 72.2 ms · crossed to UI 0.0 ms · indexed 0.2 ms
- 60 keystrokes into the composer: 60 frames; update ms p50 0.43 p95 0.95 max 1.52; encode ms p50 0.17 p95 0.27 max 0.59; 0 crossings carrying 0 updates
- stream 400 words at 80/s, at the foot: 425 frames; update ms p50 0.67 p95 1.80 max 6.64; encode ms p50 0.12 p95 0.33 max 0.69; 430 crossings carrying 430 updates; over 6.4 s
- stream 1200 words as fast as the Server sends, at the foot: 435 frames; update ms p50 1.07 p95 2.46 max 4.15; encode ms p50 0.14 p95 0.35 max 0.67; 1233 crossings carrying 1233 updates; over 3.9 s
- stream 400 words at 80/s, scrolled back four pages: 425 frames; update ms p50 0.87 p95 2.18 max 2.87; encode ms p50 0.17 p95 0.45 max 0.85; 429 crossings carrying 429 updates; over 6.3 s
- rss at the end 136 MB
### window, bridge Poll, rss at rest 99 MB
- open: asked→first frame encoded 100.8 ms, →on the display 123.1 ms; rss 123 MB
- 1068 rows (28 Messages, 1040 Activities) · asked→snapshot decoded 81.7 ms · crossed to UI 0.1 ms · indexed 0.2 ms
- 60 keystrokes into the composer: 60 frames; update ms p50 0.89 p95 2.37 max 2.57; encode ms p50 0.25 p95 0.66 max 0.71; 0 crossings carrying 0 updates
- stream 400 words at 80/s, at the foot: 426 frames; update ms p50 1.19 p95 2.86 max 4.07; encode ms p50 0.17 p95 0.47 max 1.33; 430 crossings carrying 430 updates; over 6.4 s
- stream 1200 words as fast as the Server sends, at the foot: 433 frames; update ms p50 1.09 p95 3.01 max 4.10; encode ms p50 0.13 p95 0.30 max 0.80; 1233 crossings carrying 1233 updates; over 3.9 s
- stream 400 words at 80/s, scrolled back four pages: 426 frames; update ms p50 0.82 p95 2.26 max 6.67; encode ms p50 0.16 p95 0.48 max 0.76; 429 crossings carrying 429 updates; over 6.4 s
- rss at the end 133 MB

## The same, bridge Batch, with the scroll stepped

### window, bridge Batch, rss at rest 99 MB
- open: asked→first frame encoded 98.8 ms, →on the display 117.2 ms; rss 120 MB
- open, its frames: 2 frames; update ms p50 19.30 p95 19.30 max 19.30; encode ms p50 0.30 p95 0.30 max 0.30; 1 crossings carrying 0 updates
- 1068 rows (28 Messages, 1040 Activities) · asked→snapshot decoded 78.8 ms · crossed to UI 0.0 ms · indexed 0.2 ms
- stepped head→foot, 240 pt every 8 ms: 878 frames; update ms p50 0.96 p95 3.07 max 15.63; encode ms p50 0.11 p95 0.34 max 1.10; 0 crossings carrying 0 updates
- stepped foot→head: 997 frames; update ms p50 0.88 p95 2.70 max 4.61; encode ms p50 0.10 p95 0.30 max 0.70; 0 crossings carrying 0 updates; rss 147 MB
- 60 keystrokes into the composer: 60 frames; update ms p50 0.90 p95 1.34 max 2.10; encode ms p50 0.22 p95 0.40 max 0.81; 0 crossings carrying 0 updates
- stream 400 words at 80/s, at the foot: 425 frames; update ms p50 0.76 p95 1.92 max 2.65; encode ms p50 0.14 p95 0.33 max 8.25; 426 crossings carrying 430 updates; over 6.2 s
- stream 1200 words as fast as the Server sends, at the foot: 445 frames; update ms p50 0.80 p95 1.10 max 1.41; encode ms p50 0.11 p95 0.15 max 0.21; 1232 crossings carrying 1233 updates; over 3.9 s
- stream 400 words at 80/s, scrolled back four pages: 425 frames; update ms p50 0.42 p95 0.46 max 1.31; encode ms p50 0.10 p95 0.13 max 0.19; 429 crossings carrying 429 updates; over 6.3 s
- rss at the end 153 MB
