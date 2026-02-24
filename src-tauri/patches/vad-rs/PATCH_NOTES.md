# vad-rs patch

Vendored from [upstream](https://github.com/cjpais/vad-rs) to align
`ndarray` and `ort` versions with the versions used by `transcribe-rs`.
Without this patch, Cargo resolves conflicting versions that fail to link.

Remove once vad-rs publishes with ndarray 0.17 / ort 2.0.0-rc.11
compatibility.
