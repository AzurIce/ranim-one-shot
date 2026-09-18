+++
title = "run1-glm53flash"
template = "run.html"
description = "用五种常用 3×3 卷积核（盒式模糊、锐化、Laplacian 边缘、Sobel X/Y）演示图像卷积：内置真实卷积实现驱动滑动窗口扫描与逐格显现"
[extra]
topic = "convolution_kernels"
[extra.model]
name = "GLM-5.3-Flash"
id = "builtin:bigmodel-coding-plan/GLM-5.3-Flash"
source = "self-report"
[extra.harness]
name = "ZCode CLI"
version = "unknown"
[extra.protocol]
ref = "base@bb07e7c2"
commit = "bb07e7c2b6a0228f2cdf68d5e15ffaa06864f520"
skill_modified = false
[extra.run]
date = "2026-09-18"
render_rounds = 5
wall_time = "1h20m"
[extra.delivery]
pin = "40d15be64edf5c04a78e2db75908e4a192a8e942"
duration_s = 98
duration_min = 1.6
resolution = "1920x1080@60"
tests = 10
video = "https://azurice-shadow.tos-cn-beijing.volces.com/ranim-one-shot/objects/sha256/88/62c52738f44a1a9311045fcc9c4b27582ffa84f2b4a26633bdb7d7507594d4"
previews = ["runs/convolution_kernels/run1-glm53flash/mechanism.png", "runs/convolution_kernels/run1-glm53flash/numbers.png", "runs/convolution_kernels/run1-glm53flash/preview.png", "runs/convolution_kernels/run1-glm53flash/scan_box.png", "runs/convolution_kernels/run1-glm53flash/scan_edge.png", "runs/convolution_kernels/run1-glm53flash/scan_sobelx.png", "runs/convolution_kernels/run1-glm53flash/scan_sobely.png"]
+++
