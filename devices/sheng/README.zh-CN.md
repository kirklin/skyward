# Xiaomi Pad 6S Pro 12.4（sheng）Skyward Pad 实验适配

基于 Skyward 0.1.4，工作分支为 `Pad`。官方镜像基线为
`OS3.0.303.0.WNXCNXM`（Android 16），沿用官方 recovery 的 init、ADB、
fastbootd、库和 SELinux 策略文件，替换 recovery 程序并加入 sheng 配置和启动脚本。

## 已验证和待验证

2026-11-07 已在用户的澎湃系统平板上启动 Skyward，验证显示、触摸、ADB、
电池读取、ADSP、用户 0 文件解密和 MTP。用户测试过备份和 USB 盘读取。
已修复首屏出现后触摸仍需等待数秒的问题：DRM 启用面板后，通过
`/proc/xm_htc_sw_reset` 提前加载控制器的运行内存固件，沿用设备现有固件文件，
不修改固件分区。实测加载约 100 ms，首屏后约 70 ms 即收到触摸；用户确认立即可点。

WLAN 已验证扫描、关联、DHCP 获取 IPv4 地址及联网检测。修复包括：可选的
persist MAC 文件不存在时继续使用驱动提供的地址；DHCP 等待 ACK 时忽略重复 OFFER；
从 fastbootd 返回时，在挂载系统分区后才初始化 WLAN，正确取得 VNDK 库路径。
冷启动及 fastbootd 返回后均已验证获取 IPv4 地址和联网检测。
不向 persist 写入或伪造 MAC。

平板 UI：左侧约 1/3 常驻原风格的彩色图标与圆角分组卡片，右侧约 2/3
显示功能页；默认显示关于本机，灰色标记当前项并显示按下反馈，状态栏覆盖全屏。
浅色/深色模式预览卡片限制尺寸。竖屏和窄屏继续采用原单栏布局。
fastbootd 单独保留原版全屏页面，不显示侧栏；返回 recovery 后恢复双栏及关于本机。
已验证 fastbootd 的 userspace、product、current-slot 和分区大小只读查询。

本机 `supports_vibration = false`，隐藏振动设置、搜索结果并禁用反馈；其他设备
可以独立配置。最低点亮背光为 raw 1，正常上限为 4095；熄屏仍为 0。
电池典型容量显示 10000 mAh，详细页保留驱动设计容量和当前满充容量。

实际测试系统为用户修改的 `OS4.0.2.0.XPYCNXM_peiyuesheng`、Android 17，
内核 `5.15.211-peiyuesheng-sm8550-kernel-android13+`，vendor 为 Android 13/VNDK 33。
当前 vendor_boot 与官方 303 对应内容一致。不能据此保证所有 ROM、内核版本都兼容。

完整刷包、备份恢复写入、fastbootd 重写分区等仍待独立验证。本适配不以
清除数据或刷写其他分区作为功能测试。

## 启动和设备配置

原生横屏 3048×2032、密度 400。触摸使用 nt36532_touch 和 ODM 的
`touch_report_debug`，交换 X/Y 后翻转 Y，并忽略两个具体笔输入节点。
旧 TWRP 设备树用于核对坐标、模块、固件路径、背光和 ADSP 启动顺序，
不复用其界面、Magisk 附件或 AVB 修改工具。

启动脚本只读挂载 modem 到 `/firmware`，提供原厂兼容路径，单次启动 ADSP，
等待电池节点后启动服务。QSEE、KeyMint、Gatekeeper、hwservicemanager 和
BootControl 使用设备已有系统上的匹配服务和库。

官方 recovery 为 header v4、无内核、100 MiB，沿用当前槽位 boot/vendor_boot
中的内核和 DTB。镜像带重新生成的 unsigned AVB hash footer，需要解锁 bootloader。
它不适合直接作为 `fastboot boot` 的独立有内核镜像。

## 本地构建和审计

GitHub Actions 手动运行时，选择 `device = sheng`，并通过
`stock_recovery_url` 提供官方 recovery 的 HTTPS 下载地址。工作流会校验镜像
SHA-256 并应用 sheng 的 ramdisk overlay；默认自动构建列表排除需要手动提供
官方 recovery 的机型。本地构建不需要上传官方镜像。

Linux/WSL 需安装仓库要求的 Rust、aarch64-linux-android 目标、NDK r29、
cargo-ndk 和 lz4，并设置 `ANDROID_NDK_HOME`。

```sh
bash tools/build-sheng.sh /只读官方包/images/recovery.img target/sheng-pad
```

脚本必须应用 `devices/sheng/ramdisk`，并核对官方 recovery SHA-256：
`4b66cca647e21929870dfcc38c5915e35c004016bbe815bb91a87239230d649f`。
所有输出写入指定构建目录；官方包仅供读取。

```sh
python3 tools/verify_sheng_image.py target/sheng-pad/镜像.img \
  --stock /只读官方包/images/recovery.img --binary /构建目录/skyward \
  --packer /构建目录/skyward-pack
```

提取工具仅导出 super 内的 `odm_a` 和 `vendor_a`，结束后删除自行产生的
临时 raw super，保留官方原始 super.img。文件系统建议解到 Linux 原生目录。

```sh
python3 tools/extract_sheng_stock.py --images /只读官方包/images \
  --out target/stock/sheng303 --filesystem-out /Linux原生目录/sheng303
```

发布包包含镜像、SHA256SUMS、本说明和来源记录，不包含自动刷写脚本。
运行日志在 `/skyward/run/recovery.log` 和 `/skyward/run/sheng-hardware.log`。
ADB 在 recovery 下以 root 运行。电脑预览仅验证布局，不能替代真机验证。
