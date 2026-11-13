# Redmi K60（mondrian）v0.1.6 测试版

发布日期：2026-11-13。此版本面向下述官改包测试环境，属于非官方适配测试版。

## 下载与源码

- [下载镜像、SHA256SUMS 与中文更新日志](https://github.com/Memories-white/skyward/releases/tag/mondrian-v0.1.6-os4-test1)
- [向原仓库提交的 PR #26](https://github.com/kirklin/skyward/pull/26)
- 上游基线：Skyward `v0.1.6`，提交 `3c5b0a2edb03c9aafebfb241380f4df8aeb4851b`。
- 镜像构建源码：`feat/mondrian-recovery` 分支的提交 `a84f5f6740d444ad8c1f8abd8e3fb330fd3ce93e`。本次发布使用此前刷机验证的原镜像，后续发布文档提交不改变该镜像。

| 项目 | 内容 |
| --- | --- |
| 文件名 | `skyward-mondrian-v0.1.6-OS4.0.6.0-test.img` |
| 大小 | 104857600 字节（100 MiB） |
| SHA-256 | `14b39267921a5dbb73bc8dad4b265e40d560a9c468337f127ca871dd98f40004` |

## 中文更新日志

相较此前基于 Skyward v0.1.2 的 K60 测试版，本次将设备适配移植到上游 v0.1.6，并保留以下适配和修复：

- 配置 K60 的 Goodix 触摸驱动重载与 `touch_report` 服务。
- 配置 KeyMint、Gatekeeper 和 QSEE 服务，并在原厂 recovery 缺少 AIDL service manager 时使用已安装系统的服务管理器，适配其 Binder 请求头。
- 修正 VINTF 目录权限，避免 `hwservicemanager` 因无法读取清单而禁用；补充服务启动失败日志。
- 配置 `modemst1`、`modemst2`、`fsg`、`fsc`、`persist` 五个 EFS 备份分区，拒绝空备份计划和空清单，避免空备份被当作成功。
- 配置 Boot Control HAL，供重启页面查询启动槽位。
- 配置 Qualcomm 振动设备的共振频率读取路径；实际振感仍待确认。
- 沿用上游 v0.1.6 的设置持久化实现，保存路径为 `/metadata/skyward/settings.json`。旧测试版的 `settings.toml` 不会自动迁移，升级后需重新检查设置。

## 测试环境

| 项目 | 测试记录 |
| --- | --- |
| 手机 | Redmi K60，代号 `mondrian`，型号 `23013RK75C` |
| Bootloader / 槽位 | 已解锁 / A |
| 已安装系统 | 非官方 HyperOS 官改包 `OS4.0.6.0.XPCCNXM` |
| Android | Android 17，SDK 37 |
| 内核 | `5.10.237-Fuutao-Qn_miao-source-gc28e50058` |
| recovery ramdisk 基底 | K60 官方 `OS3.0.8.0.VMNCNXM` |
| 实际刷入分区 | `recovery_a` |

这不是 K60 官方 OS4 recovery。镜像使用官方 OS3 recovery ramdisk，启动和功能依赖手机已安装的 OS4 官改包所提供的内核、vendor_boot 及系统/vendor 服务。其他 ROM 和内核组合尚未验证。

## 验证记录与限制

以下真机结果来自 2026-11-08 的测试记录；2026-11-13 发布时重新计算了本地镜像 SHA-256，确认文件未变。

| 验证项 | 本次 v0.1.6 镜像的结果 |
| --- | --- |
| 写入与回读 | 已刷入 `recovery_a`，整分区 SHA-256 与上方镜像一致 |
| 启动 | 已进入 Skyward，运行日志显示 `mondrian 0.1.6 recovery up` |
| 服务 | 观察到 `touch_report`、KeyMint、Gatekeeper、Boot Control 和 MTP 服务运行 |
| 设置文件 | 观察到 `settings.json` 已保存，`/metadata` 恢复只读；跨重启持久化尚未在此版本复测 |
| 触摸操作、锁屏密码解密 data、EFS 备份 | 此版本尚未逐项复测 |
| 实际振动、重启页面“下次启动”显示、备份恢复 | 尚未完成验证 |

此前 v0.1.2 适配测试过程中，已确认触摸操作、锁屏密码解密、设置跨重启保留，以及五个分区共 89 MiB 的 EFS 备份。这些结果属于旧版测试，不能作为 v0.1.6 对应功能已通过的证明；未做恢复测试。

## 校验与诊断

下载后先按 `SHA256SUMS` 校验镜像。刷写后还必须读取实际目标 recovery 整分区并核对 SHA-256：本机曾出现 fastboot 报告成功但仍运行旧镜像的情况。

在该测试机上不要运行 Skyward 自带的 `recovery info`、`recovery load` 或 `recovery slot` 等命令，曾导致 ADB 断开。诊断使用 ADB 读取日志；v0.1.6 未生成旧版的 `/tmp/recovery.log` 时，可读取 `dmesg`。解密密码仅在手机界面输入。
