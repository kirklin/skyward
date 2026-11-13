/*
 * Copyright (C) 2008 The Android Open Source Project
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 *  * Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 *  * Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in
 *    the documentation and/or other materials provided with the
 *    distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
 * "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
 * LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS
 * FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE
 * COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT,
 * INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING,
 * BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS
 * OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED
 * AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
 * OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT
 * OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
 * SUCH DAMAGE.
 */

#ifndef _SYS_TYPES_H_
#define _SYS_TYPES_H_

#include <sys/cdefs.h>

#include <stddef.h>
#include <stdint.h>

#include <linux/types.h>
#include <linux/posix_types.h>

#include <bits/pthread_types.h>

__BEGIN_DECLS

/* gids, uids, and pids are all 32-bit. */
typedef __kernel_gid32_t __gid_t;
typedef __gid_t gid_t;
typedef __kernel_uid32_t __uid_t;
typedef __uid_t uid_t;
typedef __kernel_pid_t __pid_t;
typedef __pid_t pid_t;
typedef uint32_t __id_t;
typedef __id_t id_t;

typedef unsigned long blkcnt_t;
typedef unsigned long blksize_t;
typedef __kernel_caddr_t caddr_t;
typedef __kernel_clock_t clock_t;

typedef __kernel_clockid_t __clockid_t;
typedef __clockid_t clockid_t;

typedef __kernel_daddr_t daddr_t;
typedef unsigned long fsblkcnt_t;
typedef unsigned long fsfilcnt_t;

typedef __kernel_mode_t __mode_t;
typedef __mode_t mode_t;

typedef __kernel_key_t __key_t;
typedef __key_t key_t;

typedef __kernel_ino_t __ino_t;
typedef __ino_t ino_t;

typedef uint64_t ino64_t;

typedef uint32_t __nlink_t;
typedef __nlink_t nlink_t;

typedef void* __timer_t;
typedef __timer_t timer_t;

typedef __kernel_suseconds_t __suseconds_t;
typedef __suseconds_t suseconds_t;

/* useconds_t is 32-bit on both LP32 and LP64. */
typedef uint32_t __useconds_t;
typedef __useconds_t useconds_t;

#if !defined(__LP64__)
/* This historical accident means that we had a 32-bit dev_t on 32-bit architectures. */
typedef uint32_t dev_t;
#else
typedef uint64_t dev_t;
#endif

/* This historical accident means that we had a 32-bit time_t on 32-bit architectures. */
typedef __kernel_time_t __time_t;
typedef __time_t time_t;

/* This historical accident means that we had a 32-bit off_t on 32-bit architectures. */
/* See https://android.googlesource.com/platform/bionic/+/main/docs/32-bit-abi.md */
#if defined(__USE_FILE_OFFSET64) || defined(__LP64__)
typedef int64_t off_t;
typedef off_t loff_t;
typedef loff_t off64_t;
#else
typedef __kernel_off_t off_t;
typedef __kernel_loff_t loff_t;
typedef loff_t off64_t;
#endif

#if !defined(__LP64__)
/* This historical accident means that we had a signed socklen_t on 32-bit architectures. */
typedef int32_t __socklen_t;
#else
/* LP64 still has a 32-bit socklen_t. */
typedef uint32_t __socklen_t;
#endif
typedef __socklen_t socklen_t;

typedef __builtin_va_list __va_list;

/**
 * A signed alternative to size_t,
 * generally for cases that return -1 and set errno on error.
 */
typedef __kernel_ssize_t ssize_t;

/** BSD synonym for unsigned int that's always exposed by historical accident. */
typedef unsigned int        uint_t;
/** BSD synonym for unsigned int that's always exposed by historical accident. */
typedef unsigned int        uint;

/** BSD synonym for unsigned char that's always exposed by historical accident. */
typedef unsigned char  u_char;
/** BSD synonym for unsigned short that's always exposed by historical accident. */
typedef unsigned short u_short;
/** BSD synonym for unsigned int that's always exposed by historical accident. */
typedef unsigned int   u_int;
/** BSD synonym for unsigned long that's always exposed by historical accident. */
typedef unsigned long  u_long;

/** BSD synonym for uint32_t that's always exposed by historical accident. */
typedef uint32_t u_int32_t;
/** BSD synonym for uint16_t that's always exposed by historical accident. */
typedef uint16_t u_int16_t;
/** BSD synonym for uint8_t that's always exposed by historical accident. */
typedef uint8_t  u_int8_t;
/** BSD synonym for uint64_t that's always exposed by historical accident. */
typedef uint64_t u_int64_t;

__END_DECLS

#endif
