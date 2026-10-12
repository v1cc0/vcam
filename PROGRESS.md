# Progress

## 本轮完成

- `vcam --preview` 右侧接入选定 `/dev/video*` 的 V4L2 控件：brightness、contrast、saturation、hue、white balance automatic、gamma、gain、power line frequency、white balance temperature、sharpness、backlight compensation、auto exposure、exposure time。
- 控件值通过 `v4l` 直接写回设备；`backlight_compensation` 按设备实际整数 `0/1` 写入，自动白平衡仍按 Boolean 写入。
- 右侧面板分为可折叠的 Camera controls、Video filters、Software effects；预览滤镜支持 `none`、`transpose=1`、`transpose=2`、`hflip`、`vflip`。
- 处理了 GPUI 滑块的非默认范围初始化：高于 `100` 的最小值先设置最大值，低于 `0` 的范围反向设置；V4L2 反向元数据范围统一排序。
- 恢复右侧 `320px` 固定宽度，避免 Accordion 横向撑开并覆盖中间原始/处理双预览。
- 保留原始/处理图像资源替换前的 `Window::drop_image` 回收逻辑。

## 验证

- `cargo fmt --all -- --check` 通过。
- `cargo check` 通过。
- `cargo test`：21 项全部通过。
- `git diff --check` 通过。
- `/dev/video1`、`640x480@15` 真实 Wayland smoke test 启动无 panic、无日志错误，帧数持续增长；截图确认左侧设备栏、中间上下双预览和右侧固定控制栏互不覆盖。
- niri 识别窗口 App ID：`com.v1cc0.vcam`。
- 预览进程、截图和日志临时文件已清理。

## 下一步

- 增加设备选择、源格式/分辨率/FPS 选择、截图、录制和输出到 loopback 的交互。
- 对 GPUI 图像上传做更长时间 CPU/内存测量；此前的旧图像 atlas 累积已由 `Window::drop_image` 修复。

## 用户文件

- `background.pgm` 保持未跟踪、未修改，不加入提交。
- `PROGRESS.md.lastround` 是本轮本地备份，保持忽略。
