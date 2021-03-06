mod err;
pub(super) use connection::*;
pub use err::RedisConnErr;
#[cfg(any(test, feature = "bench"))]
pub(self) use mock_connection as connection;

#[cfg(not(any(test, feature = "bench")))]
mod connection {
    use super::super::Error as ManagerErr;
    use super::super::RedisCmd;
    use super::err::RedisConnErr;
    use crate::config::Redis;
    use crate::request::Timeline;

    use futures::{Async, Poll};
    use lru::LruCache;
    use std::io::{self, Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    type Result<T> = std::result::Result<T, RedisConnErr>;

    #[derive(Debug)]
    pub struct RedisConn {
        primary: TcpStream,
        secondary: TcpStream,
        pub(in super::super) namespace: Option<String>,
        // TODO: eventually, it might make sense to have Mastodon publish to timelines with
        //       the tag number instead of the tag name.  This would save us from dealing
        //       with a cache here and would be consistent with how lists/users are handled.
        pub(in super::super) tag_name_cache: LruCache<i64, String>,
        pub(in super::super) input: Vec<u8>,
    }

    impl RedisConn {
        pub(in super::super) fn new(redis_cfg: &Redis) -> Result<Self> {
            let addr = [&*redis_cfg.host, ":", &*redis_cfg.port.to_string()].concat();

            let conn = Self::new_connection(&addr, redis_cfg.password.as_ref())?;
            conn.set_nonblocking(true)
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            Ok(Self {
                primary: conn,
                secondary: Self::new_connection(&addr, redis_cfg.password.as_ref())?,
                tag_name_cache: LruCache::new(1000),
                namespace: redis_cfg.namespace.clone().0,
                input: vec![0; 4096 * 4],
            })
        }

        pub(in super::super) fn poll_redis(&mut self, i: usize) -> Poll<Option<usize>, ManagerErr> {
            const BLOCK: usize = 4096 * 2;
            if self.input.len() < i + BLOCK {
                self.input.resize(self.input.len() * 2, 0);
                log::info!("Resizing input buffer to {} KiB.", self.input.len() / 1024);
                // log::info!("Current buffer: {}", String::from_utf8_lossy(&self.input));
            }

            use Async::*;
            match self.primary.read(&mut self.input[i..i + BLOCK]) {
                Ok(n) if n == 0 => Ok(Ready(None)),
                Ok(n) => Ok(Ready(Some(n))),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock) => Ok(NotReady),
                Err(e) => {
                    Ready(log::error!("{}", e));
                    Ok(Ready(None))
                }
            }
        }

        pub(crate) fn send_cmd(&mut self, cmd: RedisCmd, timelines: &[Timeline]) -> Result<()> {
            let namespace = self.namespace.take();
            let timelines: Result<Vec<String>> = timelines
                .iter()
                .map(|tl| {
                    let hashtag = tl.tag().and_then(|id| self.tag_name_cache.get(&id));
                    match &namespace {
                        Some(ns) => Ok(format!("{}:{}", ns, tl.to_redis_raw_timeline(hashtag)?)),
                        None => Ok(tl.to_redis_raw_timeline(hashtag)?),
                    }
                })
                .collect();

            let (primary_cmd, secondary_cmd) = cmd.into_sendable(&timelines?[..]);
            self.primary.write_all(&primary_cmd)?;

            // We also need to set a key to tell the Puma server that we've subscribed or
            // unsubscribed to the channel because it stops publishing updates when it thinks
            // no one is subscribed.
            // (Documented in [PR #3278](https://github.com/tootsuite/mastodon/pull/3278))
            // Question: why can't the Puma server just use NUMSUB for this?
            self.secondary.write_all(&secondary_cmd)?;
            Ok(())
        }

        fn new_connection(addr: &str, pass: Option<&String>) -> Result<TcpStream> {
            let mut conn = TcpStream::connect(&addr)?;
            if let Some(password) = pass {
                Self::auth_connection(&mut conn, &addr, password)?;
            }

            Self::validate_connection(&mut conn, &addr)?;
            conn.set_read_timeout(Some(Duration::from_millis(10)))
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            Self::set_connection_name(&mut conn, &addr)?;
            Ok(conn)
        }

        fn auth_connection(conn: &mut TcpStream, addr: &str, pass: &str) -> Result<()> {
            conn.write_all(
                &[
                    b"*2\r\n$4\r\nauth\r\n$",
                    pass.len().to_string().as_bytes(),
                    b"\r\n",
                    pass.as_bytes(),
                    b"\r\n",
                ]
                .concat(),
            )
            .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            let mut buffer = vec![0_u8; 5];
            conn.read_exact(&mut buffer)
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            if String::from_utf8_lossy(&buffer) != "+OK\r\n" {
                Err(RedisConnErr::IncorrectPassword(pass.to_string()))?
            }
            Ok(())
        }

        fn validate_connection(conn: &mut TcpStream, addr: &str) -> Result<()> {
            conn.write_all(b"PING\r\n")
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            let mut buffer = vec![0_u8; 100];
            conn.read(&mut buffer)
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            let reply = String::from_utf8_lossy(&buffer);
            match &*reply {
                r if r.starts_with("+PONG\r\n") => Ok(()),
                r if r.starts_with("-NOAUTH") => Err(RedisConnErr::MissingPassword),
                r if r.starts_with("HTTP/1.") => Err(RedisConnErr::NotRedis(addr.to_string())),
                _ => Err(RedisConnErr::InvalidRedisReply(reply.to_string())),
            }
        }

        fn set_connection_name(conn: &mut TcpStream, addr: &str) -> Result<()> {
            conn.write_all(b"*3\r\n$6\r\nCLIENT\r\n$7\r\nSETNAME\r\n$8\r\nflodgatt\r\n")
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            let mut buffer = vec![0_u8; 100];
            conn.read(&mut buffer)
                .map_err(|e| RedisConnErr::with_addr(&addr, e))?;
            let reply = String::from_utf8_lossy(&buffer);
            match &*reply {
                r if r.starts_with("+OK\r\n") => Ok(()),
                _ => Err(RedisConnErr::InvalidRedisReply(reply.to_string())),
            }
        }
    }
}
#[cfg(any(test, feature = "bench"))]
mod mock_connection {
    use super::super::Error as ManagerErr;
    use super::super::RedisCmd;
    use super::err::RedisConnErr;
    use crate::config::Redis;
    use crate::request::Timeline;

    use futures::{Async, Poll};
    use lru::LruCache;
    use std::collections::VecDeque;

    type Result<T> = std::result::Result<T, RedisConnErr>;

    #[derive(Debug)]
    pub struct RedisConn {
        pub(in super::super) namespace: Option<String>,
        pub(in super::super) tag_name_cache: LruCache<i64, String>,
        pub(in super::super) input: Vec<u8>,
        pub(in super::super) test_input: VecDeque<u8>,
    }

    impl RedisConn {
        pub(in super::super) fn new(redis_cfg: &Redis) -> Result<Self> {
            Ok(Self {
                tag_name_cache: LruCache::new(1000),
                namespace: redis_cfg.namespace.clone().0,
                input: vec![0; 4096 * 4],
                test_input: VecDeque::new(),
            })
        }

        pub fn poll_redis(&mut self, start: usize) -> Poll<Option<usize>, ManagerErr> {
            const BLOCK: usize = 4096 * 2;
            if self.input.len() < start + BLOCK {
                self.input.resize(self.input.len() * 2, 0);
                log::info!("Resizing input buffer to {} KiB.", self.input.len() / 1024);
            }

            for i in 0..BLOCK {
                if let Some(byte) = self.test_input.pop_front() {
                    self.input[start + i] = byte;
                } else if i > 0 {
                    return Ok(Async::Ready(Some(i)));
                } else {
                    return Ok(Async::Ready(None));
                }
            }
            Ok(Async::Ready(Some(BLOCK)))
        }

        pub fn add(&mut self, input: &[u8]) {
            for byte in input {
                self.test_input.push_back(*byte)
            }
        }
        pub(crate) fn send_cmd(&mut self, cmd: RedisCmd, timelines: &[Timeline]) -> Result<()> {
            // stub - does nothing; silences some unused-code warnings
            let timelines: Result<Vec<String>> = timelines
                .iter()
                .map(|tl| Ok(tl.to_redis_raw_timeline(None).expect("test")))
                .collect();

            let _ = cmd.into_sendable(&timelines?);

            Ok(())
        }
    }
}
