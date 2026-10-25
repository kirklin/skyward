# se_omapi

A small native server for `android.se.omapi.ISecureElementService/default`
that forwards every channel to the phone's secure element HAL
(`android.hardware.secure_element.ISecureElement/eSE1`). A recovery has
no Java side to serve OMAPI, and NXP's weaver service reaches the eSE
applet through OMAPI alone — its transport is chosen at build time — so
a recovery that means to decrypt on a phone with an NXP eSE needs this
proxy running.

Carried as a prebuilt from the OrangeFox device tree for the Redmi K90
Pro Max (myron), which serves the same purpose the same way
(GPL): https://github.com/haohao3001/OrangeFox-REDMI-K90ProMax-myron

The equivalent binary in the TWRP tree for this platform, differently
built but answering the same interface, is at:
https://github.com/MissMyTime/twrp_device_sm8850

The original author and build recipe are not known to me; if you know
where it is built from, replace this prebuilt with a build from source.

- size: 103488 bytes
- MD5: 7278edd70ceea428eac85c2e8dffdd6f
- SHA-256: 6b03fd85d7b1032f33bce4cbb083f47b0aa0b20ea6f0fcf3a16e7e73ea84c604

## Packing it

```sh
target/release/skyward-pack recovery ... \
  --extra system/bin/se_omapi=resources/bin/se_omapi \
  --out target/skyward-<codename>.img
```
