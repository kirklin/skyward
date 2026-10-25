#!/usr/bin/env python3
"""Transplant a stock recovery's signed AVB tail onto a custom recovery
image — 适配"假回锁"（fake relock）：让自编译 recovery 带上官方 vbmeta
签名块，引导校验按官方镜像对待。

What this does / 做了什么:

An Android recovery partition ends in an AVB footer whose pointers name a
vbmeta blob elsewhere in the partition. A recovery built by skyward-pack
ends in a footer of its own over an *unsigned* blob; a stock recovery
ends in the phone maker's signature over the stock payload. This script
takes the maker's signed vbmeta blob and footer out of a stock recovery
image and re-dresses a custom image with them: the custom payload is kept
byte for byte, the signed blob is written in behind it, the pointers are
rebuilt to the new offsets, and the partition is padded out to the same
size with zeros. The result passes the bootloader's check of the recovery
partition exactly as a stock image does, which is what a phone wants
before its bootloader is locked again (the "fake relock" 假回锁 practice:
the partition reads as official even though it carries a custom
recovery).

Android recovery 分区以一个 AVB footer 结尾，其中的指针指向分区内的
vbmeta 签名块。skyward-pack 构建的镜像自带的是未签名块；官方镜像带的
是厂商对官方内容的签名。本脚本把官方镜像里的签名 vbmeta 块连同 footer
取出来，穿到自定义镜像上：自定义内容逐字节保留，签名块紧贴其后，
footer 指针按新偏移重建，剩余空间零填充到与分区等大。产物能像官方
镜像一样通过引导对 recovery 分区的校验——即"假回锁"场景所需。

Usage / 用法:

    python transplanting_vbmeta.py <官方recovery.img> <自定义镜像.img> <输出.img>

The stock image must be the whole partition, ending in its AVB footer,
as the phone or a full update package holds it. The custom image may end
in a footer of skyward-pack's own, which is stripped; one that does not
is taken whole. A custom payload too large for the partition beside the
signed blob is refused.

Source / 来源: carried unchanged in logic from the OrangeFox device tree
for this phone, where it serves the same purpose:
https://github.com/haohao3001/OrangeFox-REDMI-K90ProMax-myron/blob/main/scripts/transplanting_vbmeta.py
(only these comments are added here).

Note / 注意: the signature is over the *stock* payload, so it says nothing
about the custom one — this dresses a custom image as stock for the
bootloader's partition check, it does not sign the custom image itself.
"""
import os
import struct
import sys

def transplant_avb_smart(official_img, custom_img, output_img):
    print("[*] 正在解析镜像并进行智能移植...")

    # 1. 提取官方 VBMeta 签名块
    #    The stock image's last 64 bytes are its AVB footer: the magic
    #    "AVBf", the version, the original image size, and where the
    #    signed vbmeta blob sits and how long it is. Read the footer, then
    #    the blob it names.
    with open(official_img, 'rb') as f_off:
        f_off.seek(0, os.SEEK_END)
        part_size = f_off.tell()

        f_off.seek(part_size - 64)
        footer = f_off.read(64)
        magic, major, minor, orig_size, vbmeta_offset, vbmeta_size = struct.unpack('>4sLLQQQ', footer[:36])

        if magic != b'AVBf':
            print("[-] 错误：官方镜像不包含 AVB 尾部！")
            sys.exit(1)

        f_off.seek(vbmeta_offset)
        off_vbmeta_blob = f_off.read(vbmeta_size)
        print(f"[+] 提取官方签名块成功 (大小: {vbmeta_size} 字节)")

    # 2. 智能剥离自定义镜像的填充和旧签名
    #    A custom image built with a footer of its own ends it over an
    #    unsigned blob: keep only the real payload in front of that blob,
    #    dropping the blob, the padding and the footer. An image without
    #    a footer is taken whole.
    with open(custom_img, 'rb') as f_cust:
        f_cust.seek(0, os.SEEK_END)
        cust_size = f_cust.tell()

        f_cust.seek(cust_size - 64)
        cust_footer = f_cust.read(64)
        magic, _, _, _, cust_vbmeta_offset, _ = struct.unpack('>4sLLQQQ', cust_footer[:36])

        f_cust.seek(0)
        if magic == b'AVBf':
            # 如果包含我们编译时加的测试签名，将其剥离，只取真实 Payload
            pure_cust_data = f_cust.read(cust_vbmeta_offset)
            print(f"[+] 成功剥离测试签名，自定义镜像真实体积: {len(pure_cust_data)} 字节")
        else:
            pure_cust_data = f_cust.read()
            print(f"[+] 未发现测试签名，自定义镜像真实体积: {len(pure_cust_data)} 字节")

    # 检查真实体积是否超限
    # The payload, the signed blob and a footer have to fit the partition.
    if len(pure_cust_data) + len(off_vbmeta_blob) + 64 > part_size:
        print("[-] 错误：自定义镜像太大了，分区装不下！请精简组件。")
        sys.exit(1)

    # 3. 动态重构 AVB 尾部指针并组装
    #    Assemble in partition order: the payload, the signed blob right
    #    behind it, zero padding up to the last 64 bytes, and a footer
    #    that points at the blob where it now sits. The footer is written
    #    with the same magic, version and blob length as the stock one;
    #    only the offsets move.
    new_vbmeta_offset = len(pure_cust_data)
    # 打包新的 64 字节 Footer
    new_footer = struct.pack('>4sLLQQQ', b'AVBf', major, minor, new_vbmeta_offset, new_vbmeta_offset, vbmeta_size)
    new_footer += b'\0' * 28  # 填充剩下的保留位

    with open(output_img, 'wb') as f_out:
        f_out.write(pure_cust_data)       # 写入真正的自定义镜像内容
        f_out.write(off_vbmeta_blob)      # 紧贴着写入官方签名块

        current_pos = f_out.tell()
        f_out.write(b'\0' * (part_size - 64 - current_pos)) # 用 0 填充满剩余的空白

        f_out.write(new_footer)           # 在最后 64 字节打上新的指针

    print(f"[+] AVB移植成功，新镜像已生成：{output_img}")

if __name__ == '__main__':
    if len(sys.argv) != 4:
        print("用法: python transplanting_vbmeta.py <原厂recovery.img> <自定义镜像> <输出文件>")
        sys.exit(1)
    transplant_avb_smart(sys.argv[1], sys.argv[2], sys.argv[3])
