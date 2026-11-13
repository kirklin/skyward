# K60 Pro 挂载开关与安装前解码错误复测

2026-11-13，从此前实测的 bb88bc7 临时镜像内存启动 Skyward。当前为 socrates／a，
启动程序及配置摘要匹配；boot_a、init_boot_a、vendor_boot_a、recovery_a 摘要未变。
本轮尚未执行新的 ROM 安装或清除／格式化操作。

机主报告挂载／卸载开关异常。将 myron 分支已验证的共用修正回移：仅卸载当前
可见文件系统，保留仍被服务使用的映射；重新挂载时严格核对原有只读 linear／zero
映射的设备、偏移和范围，完整更新清理路径仍保持严格处理。

机主尝试从界面安装现有 ZIP 后，程序在 `hold this recovery from its partition`
阶段返回 `BlockTooBig`。日志显示包被识别为脚本 ZIP，错误发生在刷包脚本启动前。
原 recovery_a 的首个 legacy LZ4 压缩块为 8,400,454 字节，合法地超过解压后
8 MiB 的块大小。lz4_flex 0.14.0 的 FrameDecoder 将压缩长度错误地限制为该
解压上限；legacy 格式即使压缩后更大，也仍必须使用压缩块。

修正为 legacy 单独使用有界流式解码：压缩长度限于 LZ4_compressBound(8 MiB)，
解压输出仍严格限于 8 MiB；截断、异常长度和损坏数据报错，并保持错误状态。
Ramdisk::decode 与安装器查找配置两条路径共用该实现；现代 LZ4 frame 路径不变。
新增合成 CPIO、参考 lz4 工具对照及异常流回归，不提交手机原始 ramdisk 数据。
同时回移仅对选定输出设备临时调整只读标志、校验后恢复的回刷修正。

## 修正后验证

e6cee97 已通过[完整云端构建和测试](https://github.com/Dingtaiqi/skyward/actions/runs/43373139184)，
包括合成大块 legacy LZ4／现代 frame／截断及损坏输入、参考 lz4 工具对照、
共享映射校验，以及真实一次性只读 loop 设备上的回刷测试。

从该 CI 镜像取出的程序在原机只读识别 recovery_a 成功。进一步调用实际
Kept::hold 路径，也完成原 recovery 的解析，正确返回“当前内存启动没有
已安装的 Skyward 可保留”；不再报 BlockTooBig，未请求写入。

使用同一已验证内核和启动参数的临时 v3 容器启动 e6cee97 后，运行程序及
配置摘要匹配，四个 a 槽启动分区摘要未变。机主确认 vendor_dlkm、mi_ext
均可卸载并重新挂载，最后已开启；电脑端也核实两者恢复为只读挂载。

| 产物 | SHA-256 |
| --- | --- |
| 原生 v4 recovery 镜像（100 MiB、无内核；本轮未实刷） | `b530d796bc09dbefbb85e7f981ddb253b7b7042fe0f3a4ad589e171640d4e9d7` |
| 实际临时启动的 v3 容器 | `eacd4408ec6a73c1ba86a9ab81a05a24cad5335db559b8a5d0904e13b46f12c3` |
| 实际运行程序 | `8a81d315c6778350974d72174139f6e543a390b1c7167bc1d0c9b5cbdec9cd58` |

整包安装没有执行。本机同名、同大小的移植 ROM 安装脚本静态检查显示，其会
写入双槽 boot/recovery 与多项 bootloader／固件分区、super、cust，清除 FRP，
并删除指定应用数据；这超出本轮启动／排查范围。手机当前共享存储尚未打开，
本轮没有为比较脚本而解密或挂载 data，因此未声称已比对手机 ZIP 的脚本摘要。
解码修复和挂载开关通过不能记作该 ROM 完整安装通过。
