use crate::from_env_var; //macro
use std::time::Duration;
//use std::{fmt, net::IpAddr, os::unix::net::UnixListener, str::FromStr, time::Duration};
//use strum_macros::{EnumString, EnumVariantNames};

from_env_var!(
    /// The host address where Redis is running
    let name = RedisHost;
    let default: String = "127.0.0.1".to_string(); 
    let (env_var, allowed_values) = ("REDIS_HOST", "any string");
    let from_str = |s| Some(s.to_string());
);
from_env_var!(
    /// The port Redis is running on
    let name = RedisPort;
    let default: u16 = 6379;
    let (env_var, allowed_values) = ("REDIS_PORT", "a number between 0 and 65535");
    let from_str = |s| s.parse().ok();
);
from_env_var!(
    /// How frequently to poll Redis
    let name = RedisInterval;
    let default: Duration = Duration::from_millis(100);
    let (env_var, allowed_values) = ("REDIS_FREQ", "a number of milliseconds");
    let from_str = |s| s.parse().map(Duration::from_millis).ok();
);
from_env_var!(
    /// The password to use for Redis
    let name = RedisPass;
    let default: Option<String> = None;
    let (env_var, allowed_values) = ("REDIS_PASSWORD", "any string");
    let from_str = |s| Some(Some(s.to_string()));
);
from_env_var!(
    /// An optional Redis Namespace
    let name = RedisNamespace;
    let default: Option<String> = None;
    let (env_var, allowed_values) = ("REDIS_NAMESPACE", "any string");
    let from_str = |s| Some(Some(s.to_string()));
);
from_env_var!(
    /// A user for Redis (not supported)
    let name = RedisUser;
    let default: Option<String> = None;
    let (env_var, allowed_values) = ("REDIS_USER", "any string");
    let from_str = |s| Some(Some(s.to_string()));
);
from_env_var!(
    /// The database to use with Redis (no current effect for PubSub connections)
    let name = RedisDb;
    let default: Option<String> = None;
    let (env_var, allowed_values) = ("REDIS_DB", "any string");
    let from_str = |s| Some(Some(s.to_string()));
);
