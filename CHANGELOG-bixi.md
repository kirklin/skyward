# Skyward for bixi · 1.0.0

> 机型：小米 MIX Flip 2（`bixi`，SM8750 / `sun`，A/B + 动态分区，官方 `OS4.0.11.XOHCNXM`）
> 上游：[kirklin/skyward](https://github.com/kirklin/skyward)；本分支：[`bixi-cover-display-v2`](https://github.com/XQAB/skyward/tree/bixi-cover-display-v2)（PR #38）

## 这是第一个可以日常用的版本

从 0.44 到 0.67，这台机器上被逐项打通的是「折叠机 recovery 到底还缺什么」：外屏、触摸、解密、存储、备份、AVB、安装脚本、更新。1.0.0 不是又一个功能版，而是把这些拼成一条**完整链路**——进 recovery、看见界面、解开 /data、备份、改 AVB、装包、更新自己，全部走得通，且每一步都在真机上验过。

里程碑式的三条：

1. **外屏成为主力屏**。合盖状态下 recovery 现在是一块能看能点的外屏界面，不再是「翻开才能用」。
2. **解密无感**。记住一次密码后，之后每次进 recovery 自动解开 `/data`，`/sdcard`、MTP、备份、装包全部随之可用。
3. **能自己更新自己**。从 GitHub 发行取镜像、sha256 校验、写进下一个槽位的 recovery 分区；加速源可选，检查与下载可以走不同的镜像。

## 重要升级（1.0.0 视角）

| 能力 | 状态 | 关键点 |
| --- | --- | --- |
| 外屏显示与触摸 | ✅ 真机验过 | 只驱动一块面板；按折叠状态切换；触摸坐标按面板重算 |
| 双击亮屏 | ✅ 真机验过（0.64） | 用面板 doze 而非关屏，触摸控制器保持工作 |
| 自动解密 / 记住密码 | ✅ 真机验过（0.65） | AES-GCM 密文存 `/metadata/skyward/credential`，密钥由序列号派生 |
| WAL 合并后读 `sp-handle` | ✅ 真机验过（0.66） | 复制 `locksettings.db` 到临时目录再打开，让 `-wal` 真的重放 |
| `/data`、`/sdcard`、MTP | ✅ | 开机后台打开 `/data` 并绑定 `/sdcard`；先解锁再开文件传输 |
| 分区镜像 | ✅ | 全备（LZ4 + `.img.lz4` + sidecar）、关键分区、最小可开机集；可存内部存储或 U 盘 |
| AVB flags 可写 | ✅ 已实现（真机验待补） | 先备份四份 `vbmeta_*`，写后上报 `0x00 -> 0x03` 这类差值 |
| 安装脚本兼容 | ✅ 真机验过 | `7za`、`zstd`、`unzip`、`avbctl`、`resetprop` 都是 recovery 内建命令 |
| 双更新源 | ✅ | 系统更新（`update.miui.com`，答复加密）+ Skyward 更新（GitHub Releases） |
| 加速源 | ✅ | 6 档可选，更新页与下载页共用；检查走能转 API 的镜像，下载走你选的镜像 |
| 自更新 | ✅ | 优先裸 `.img`，退回 `.zip`；sha256 校验；写入下一个槽位与另一块装了 Skyward 的分区 |
| 启动指令页 | ✅ 真机验过（0.66） | 读 `misc` 里的 BCB，可单独清除，不动任何分区 |
| 关于页排版 | ✅ 定案（0.63） | 字标 45.0dp、三行 17.2dp、版本 15.4dp、基线 301.0 |

## 版本时间线

| 版本 | 主题 |
| --- | --- |
| 0.44–0.47 | 外屏起步、构建与刷入链路打通；发行资产名用 `OS4.0.0.11.XOHCNXM` |
| 0.48 | 外屏显示与触摸稳定下来，第一次成套发行 |
| 0.49–0.51 | 折叠状态判定、单面板驱动策略、加速源接入下载 |
| 0.52–0.55 | 解密页、密码输入、网络与校时 |
| 0.56–0.59 | 备份页成型（全备/关键分区）、MTP、日志导出 |
| 0.60–0.62 | 双击亮屏（第一版，落点容差太严） |
| 0.63 | 「关于」页排版定案；发行资产名改为 `OS4.0.11.XOHCNXM` |
| 0.64 | 双击亮屏放宽判定，真机通过 |
| 0.65 | 记住解密密码生效（「进 rec 就自动解锁了」）；ramdisk 兼容 `.img.zst` 资产 |
| 0.66 | 镜像可存 U 盘、最小可开机集、解密页单独删、AVB 写后 flags 差值、MTP 重复 offer 封版 |
| 0.66.01 | 自更新改为优先裸 `.img`；第一次同时挂 `.img` + `.zip` 两份资产 |
| 0.67 | 「Skyward 更新」页补加速源行；加速源扩到 6 档；**检查更新走代理，与下载分开策略** |
| **1.0.0** | 汇总以上全部，版本号落定，README 与发行说明重写 |

## 逐项说明

### 外屏与显示

- 折叠机第二块面板不能与内屏同时驱动：两边都提交 atomic modeset 时，内屏 commit 会超过 2000ms 直接整机重启；另开一个 DRM fd 拿第二块屏则 `Permission denied (13)`。结论是**按折叠状态只驱动一块面板**。
- 外屏背光走 `panel1-backlight` / `dsi_display1`；只把 CRTC `ACTIVE=false` 会 `EINVAL`，必须先 `CRTC_ID=None` 解绑再关。
- 亮度滑块到 `max_brightness` 一半是正常的：`8191/16383` 就是这块屏的常规上限，不是 bug。
- 双击亮屏落地方式：熄屏走面板 doze（亮度 0）而不是真关屏，触摸控制器因此一直在工作——代价是整机不休眠，换来的才是双击能亮。

### 触摸与交互

- 触摸 IC 是 synaptics **S3910**（`/dev/tcm*`），没有任何 gesture / double-tap 节点；对 `goodix_ts` 或 synaptics 的 `tp-suspend-en/off` 直接写值会重启整机，所以双击只能自己在 UI 层做。
- 判定用「同一落点、容差放宽、抬手清第一下」，0.62 太严失败，0.64 放宽后真机通过。

### 解密与锁定

- 记住密码：成功解开 `/data` 时输入的密码被 AES-GCM 加密保存到 `/metadata/skyward/credential`，密钥由本机序列号派生，不写进设置文件；换手机解不开，关开关即删。之所以不用 TEE：recovery 无法让安全世界封装一个它没见过的密钥。
- WAL 合并：旧实现用 `file:…?immutable=1` 打开 `locksettings.db`，会忽略 `-wal`，读到的 `sp-handle` 是旧的。现在先把 `locksettings.db`（连同 `-wal`/`-shm`）复制到 `/data/local/tmp/skyward-locksettings-<pid>/` 再打开，让 SQLite 真的重放日志，实测读到了当前值。
- 解密页可以单独「删除已保存的密文」，并显示「保存于 年/月/日 时:分」。
- `user 999` 仍打不开：recovery 里 gatekeeper 起不来，这是老限制，不影响 user 0。

### 存储与传输

- 开机后台打开 `/data` 并绑定 `/sdcard`，否则 MTP 与备份都是空的。
- MTP 的顺序问题：先开「文件传输」再去解锁 `/data`，Windows 侧会看不到存储节点——必须先解锁。
- 无线 ADB 默认关闭；开启后 `adb connect 172.20.10.2:5555`，刷机前必须先 `adb disconnect`，否则双设备把脚本搞乱。

### 分区镜像与备份

- 「全部」逐个读、逐个用 LZ4 压成 `.img.lz4` 并存 sidecar `packed=lz4`；`super`、`data` 这类几个 G 的不在「全部」里。
- 「关键」= `boot`、`init_boot`、`vendor_boot`、`dtbo`、`vbmeta`、`vbmeta_system`、`persist`、`misc`。
- 「最小可开机集」= 只有 `boot`、`init_boot`、`vbmeta`、`dtbo`、`persist`、`misc` 的两槽，合起来几十兆。
- 「存到」在内部存储与 U 盘之间轮换（U 盘要插 OTG）；镜像统一落在 `Skyward/partitions`。
- 之前踩过：`persist`/`fsc`/`fsg` 这类运行时会被改写的分区，备份哈希与旧基线不一致是正常的，判断备份好坏要看它是否与**当前**分区一致。

### AVB

- 页面列出 `vbmeta_a/b`、`vbmeta_system_a/b` 的 flags 与签名算法（本机无 `vbmeta_vendor`）。
- 改动分两行：「关闭 verity 与 verification」与「恢复 verity 与 verification」，各自**点两次**（第一下上膛、第二下执行）。
- 动手前先把四份 `vbmeta_*-<时间戳>.img` 复制进镜像文件夹，可回退；写完把写前写后的 flags 差值报出来（例如 `vbmeta_a 0x00 -> 0x03`）。
- 关掉之后官方系统无法启动、官方 OTA 失败，部分应用与售后工具会读到这个状态——页面上直接写明。

### 安装

- 安装脚本里的 `7za`、`zstd`、`unzip`、`avbctl`、`resetprop` 不是外部二进制，而是 recovery 主程序按 `argv[0]` 分派的五个内建命令，ramdisk 里是五个软链。
- 脚本运行时 `PATH=/sbin:/system/bin:/system/xbin`，因此脚本里写 `7za e -so "$ZIPFILE" images/super.img.zst | zstd -d > "$OUT"` 就能直接跑通（真机实测 `7za x`、`7za e -so`、`zstd -d`、`resetprop` 读/写、`avbctl` 均可用）。
- `unzip -p` 边读边输出，避免把超大镜像整个读进内存；`zstd` 走 libzstd，支持 `-dcf` 直通。

### 更新与网络

- 两个更新源分得很清楚：
  1. 系统更新 → `https://update.miui.com/updates/miotaV3.php`，答复加密，用内置公钥解（`skyward-net/src/release.rs`）。镜像站不转这条，所以这一页**没有**加速源。
  2. Skyward 更新 → `https://api.github.com/repos/XQAB/skyward/releases/latest`，同时充当联网校时。
- 自更新的硬条件：tag 以 `v` 开头；资产名前缀 `skyward-{tag}-{codename}-`、后缀 `.zip` 或 `.img`；GitHub 给出 `sha256` digest。0.66.01 起**先找裸 `.img`，没有才退回 `.zip`**。
- 加速源 6 档：不加速 / `gh-proxy.com` / `gh-proxy.org` / `gh-proxy.cc` / `ghfast.top` / `github.akams.cn`，下载页与「Skyward 更新」页共用同一个选择，由 Keeper 定时落盘。
- **检查与下载分开**：`api.github.com` 只有 `gh-proxy.com`、`gh-proxy.org`、`gh-proxy.cc` 会转（`ghfast.top` 对接口 403、`github.akams.cn` 404）。所以检查更新会先走能转 API 的镜像，再兜底直连；下载则走你选的镜像——想要速度就选 ghfast。

### 界面与文案

- 署名三行：`Kirk Lin` / `&` / `一只小小小蜻蜓`。
- 状态栏要秒、要 USB 插线图标；键条三个一排。
- 「启动指令」页说明 `misc` 里的 BCB 是什么、清除只是抹掉那张便条。
- 鸟 log 统一 `skyward: …` 形式，导出日志页可一次取出。

### 构建

- 一个命令出整镜像：`skyward-pack recovery --stock recovery.img --binary … --system-init … --config devices/bixi.toml --font … --mono … --extra bixi_extra --out …`。
- `bixi_extra` 里 8 个文件补的是 recovery 缺的 HAL 声明与脚本（`se_omapi`、两个 NDK so、`android.se.omapi-service.xml`、`weaver-nxp.xml`、`bixi-sockets.rc`、`ueventd.rc`、`bixi-sdcard.sh`）。

## 借鉴与来源

- **上游** [kirklin/skyward](https://github.com/kirklin/skyward)：Rust 写 recovery 的整体架构、UI 框架、双系统、安装器骨架。本分支是在它之上做的机型适配。
- **aurora fork**：`AVB 校验` 页的版式与「改动」小节的分段（先准备、再执行）、BCB/启动指令页、更新页的信息层次，都参考了它的写法。原则是**只取读路径**，写路径（真正落 flags、落分区）自己重写并加备份。
- **Rinrroy/skyward-for-haotian**：折叠机外屏「只驱动一块面板」的思路参考；取源码走 `raw.githubusercontent.com` 经镜像站，避开本机 clone 不稳。
- **KernelSU 的 `resetprop`**：命令行与语义对齐它（读/写经 init、不改属性内存、`-n`/`-v` 照旧），因为安装脚本是按 KernelSU 的用法写的。
- **命令行语义**：`7za` 对齐 7-Zip（`x`/`e`/`-so`/`-o`/`-aoa`/`-aos`/`-r*`/`-bd`/`-mmt`），`zstd` 对齐 zstd 本体，`unzip` 对齐 busybox unzip 的 `-p`，`avbctl` 对齐 AOSP 的 `get-`/`disable-`/`enable-` 子命令与 `--force`。
- **AVB 规范**：flags 位（verity / verification）与「改 flags 会让签名失效」的判断来自 AVB 的公开文档。
- **镜像加速**：`gh-proxy` 系列（`gh-proxy.com` / `gh-proxy.org` / `gh-proxy.cc`）与 `ghfast.top`、`github.akams.cn`，实测数据见发行说明。

## 已知限制

- `user 999` 打不开（recovery 里 gatekeeper 起不来），不影响 user 0。
- 两块面板不能同时驱动，只能按折叠状态各驱动一块。
- 安装包只支持 zip；`7za` 目前 `x`/`e` 可用，但没有 `l`，也不认 `.7z` 容器。
- 系统「在线更新」页没有加速源（镜像站不转 MIUI 的接口）。
- 自更新的裸 `.img` 分支在 0.67.00 → 0.67.01 这一跳上跑过，1.0.0 的完整结论见发行说明下方。
- 双击亮屏的代价是熄屏时整机不再休眠。

## 资产

- `skyward-v1.0.0-bixi-OS4.0.11.XOHCNXM.img` —— 裸镜像，104857600 字节，直接写 recovery 分区。
- `skyward-v1.0.0-bixi-OS4.0.11.XOHCNXM.zip` —— 同一个镜像压成的 zip。

校验值见发行页（GitHub 会给出每个资产的 `sha256`）。
