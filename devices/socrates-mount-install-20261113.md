# K60 Pro 挂载开关与脚本安装兼容修复

2026-11-13，从此前实测的 bb88bc7 临时镜像内存启动 Skyward。当前为 socrates／a，
启动程序及配置摘要匹配；boot_a、init_boot_a、vendor_boot_a、recovery_a 摘要未变。
最初只进行临时启动和安装前诊断。随后机主通过界面执行了移植 ROM 安装，
该阶段的分区变更和校验另列在下文，不能继续沿用最初“分区未变”的结论。

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

以上为 e6cee97 阶段的验证，不能据此宣称完整 ROM 安装通过。

## 私有服务挂载导致安装前设备忙

再次从界面尝试安装时，逻辑分区卸载阶段报 `system_dlkm_a: Device or resource busy`，
脚本还未启动。实际占用者为启用了 `memory_data` 的 cnss-daemon：其私有挂载
命名空间继承了 system_dlkm，即使没有打开该分区中的文件，也仍保留设备引用。

3baaa6b 将私有命名空间的 mountinfo 设备号与逻辑分区设备的 rdev 对应，加入
既有文件锁页得出的占用集合。共享命名空间不额外保留分区，读取失败直接报错，
不忽略 EBUSY、不强制删除映射。新增 8 项回归覆盖无文件打开、共享命名空间、
其他路径的 bind、去重、无关设备和读取／格式错误。

该版本已通过[完整 CI](https://github.com/Dingtaiqi/skyward/actions/runs/43380062867)。
原机临时启动后，独立诊断调用安装器实际使用的 pin／release_partitions，再立即
恢复挂载：8 个逻辑分区均完成卸载和只读恢复，四个 a 槽启动分区摘要仍与原基线一致。
这次诊断没有执行 ROM 脚本。后续界面安装已越过这一阶段。

## 补齐真正的 Bash，提前核对解释器

界面下一次安装报 `start /tmp/update-binary: No such file or directory`。实际文件存在，
但首行为 `#!/sbin/bash`，而 recovery 只提供了 sh。其摘要与本机检查过的脚本一致。

9a21224 从固定摘要的 GNU Bash 5.3 源码和 20 个补丁，在原生 AArch64 runner 上
通过 musl 静态构建 Bash 5.3.20。打包时加入实际 Bash、GPL 许可、musl 版权和精确
源码说明；不把 sh 冒充 Bash。安装器为 `/sbin/bash` 建立指向该程序的链接，
并在保留 recovery、卸载分区和创建可写映射之前核对 shebang 解释器是否可执行。
缺少解释器时提前报告其实际路径。对应源码和构建脚本作为 `bash-source.tar.gz`
与 Actions 产物及标签 release 一同提供。

[完整 CI](https://github.com/Dingtaiqi/skyward/actions/runs/43382876856) 已通过，包含
Linux／Android Clippy、工作区回归、真实只读 loop 设备回刷测试、原厂镜像校验及
普通 socrates 镜像构建。Bash 独立构建还校验 AArch64 架构、无动态解释器／动态库，
并实测数组、局部变量、进程替换及恢复脚本的参数／输出描述符协议。
原 K60 内核下运行同一 Bash 的无写分区合成脚本也通过。

| 9a21224 产物 | SHA-256 |
| --- | --- |
| 原生 v4 recovery（100 MiB、无内核；本轮未将此候选实刷到 recovery_a） | `bf57d94232cae8490aa96deeb77bb298720ed990c20aa9eef89239a20fbea4f0` |
| 实际临时启动的 v3 容器 | `1c9f176174e355fa0fd9b9a38bfa8ee64fe0939a93d573e5ccc90a849c777122` |
| 实际运行程序 | `31e7accbf830b91c01c0b3acfdf2f4fd3be9d1a5caafc9ebfea90b1a4f548e79` |
| 实际 Bash | `43697b3c55fe5bb3e0b5fb23f004e261118deb069f2fe39cf4ae5ac2bfcb62ef` |

程序、Bash、配置及 ramdisk 中的许可／源码说明均与 CI 产物匹配。

## 机主从界面执行移植 ROM

临时启动 9a21224 后，机主从界面执行现有移植 ROM。脚本确已运行，随后报告完成
并以 0 退出。发现实际安装正在进行后，停止了独立卸载诊断，没有在写入时重启、
终止脚本或恢复旧启动分区。

该脚本会写双槽启动链／固件、super、cust，清除 FRP 并删除指定应用数据；
它并非标准 payload OTA。日志中有一次 `modem.img` 写入被拒绝，脚本仍继续执行。
安装结束后按包内镜像的实际长度比较 SHA-256：

- a 槽的各项 bootloader／固件、dtbo、vbmeta、init_boot、vendor_boot、recovery 与包内对应内容一致。
- modem_a、modem_b 的包内镜像覆盖范围也均匹配；本次无需补刷 modem。
- boot_a 匹配脚本标为“官方内核”的 `images/boot_b.img`；未代替机主按音量键选择内核。
- recovery_a 为包内 OrangeFox R11.3_260725，摘要 `0145e11d1ebf3af7b40d40c7f9f95636d159b4b68e50130b5358c4564ad2f7f1`，
  另存了与此内容一致的回退文件。本次 Skyward 从 RAM 启动，不能把运行程序摘要当作已安装分区摘要。

modem 当时仍有只读文件系统挂载。通用的 modem 拒写路径尚未修复；本次最终内容
一致不代表不同 modem 镜像也能成功覆盖。没有逐字节校验整个 super，也未据此
宣称所有第三方脚本、OTA／VAB 更新和破坏性功能已全面验证。

## 正常系统启动尚未通过

正常重启后，系统因 `fs_mgr_mount_all` 请求进入 recovery，实际进入包内橙狐。
仅凭 `sys.boot_completed=1` 的自动检查曾错误地判为系统启动完成，该记录已纠正：
此 recovery 也设置该属性，必须结合进程、构建属性和机主屏幕确认。

再次仅从 RAM 启动 9a21224 后，用关闭修复与密钥升级的只读策略查询。现有 metadata
密钥在 KeyMint 中返回 `-62`（需要升级），在解开数据密钥之前停止，未挂载 /data。
四个 metadata 密钥文件的集合与摘要仍和此前留存基线一致。另观察到系统安全补丁
从 2026-09-01 变为包内 2026-08-01；这是待核实的兼容性差异，尚不能单凭日期认定根因。
没有格式化、执行文件系统修复、回写密钥，或自动恢复旧启动分区。

8789dcf 的显式 `keymint --upgrade-memory` 工具已通过完整云端构建与检查。该选项仅允许
诊断进程在内存中对 wrapping key 重试一次 upgradeKey，不回写磁盘密钥、不挂载 data、
不改变 UI／data 的升级默认值。厂商安全世界可能改变内部状态，因此此操作等待机主
单独允许；不能将其描述为纯只读或已执行通过。
