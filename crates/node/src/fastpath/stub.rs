//! 非 Linux 平台 stub — 不会被实际调用（probe 早已返回 fastpath=false）。
//! 保留是为了让上层 spawn_forward 编译路径无 #[cfg] 散点。
