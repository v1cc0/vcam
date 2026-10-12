# Progress

## 本轮完成

- 将 `Camera controls` 从右侧移到左侧 Device/Capture/Output 信息下方，保留默认值恢复按钮和全部 V4L2 控件。
- 将中央 Original/After effects 预览从上下分屏改为左右分屏，左侧显示原始帧，右侧显示处理帧。
- 将 Video filters 合并进右侧 `Software effects`，改用 `Video rotation` 和 `Flip` 两个下拉菜单；亮度、对比度继续使用滑块。
- 旋转与翻转下拉选择会同步相互排他的状态，避免界面显示旧选项。
- 更新 README 和 PLAN；保留纯黑背景、纯白文字、GPUI 图像资源回收及 `vcam run` 行为。

## 验证

- `cargo fmt --all -- --check` 通过。
- `cargo check` 通过。
- `cargo test`：22 项全部通过，包含下拉索引映射回归测试。
- `cargo build --release` 通过。
- `git diff --check` 通过。
- `/dev/video1` release Wayland smoke test 无日志错误，niri 识别 App ID `com.v1cc0.vcam`。
- 默认状态截图确认左侧 Camera controls、中间左右双画面和右侧 Software effects 无覆盖。
- 使用 `--vf transpose=1` 启动的截图确认右侧处理画面旋转为横向显示，左右分屏保持稳定。

## 下一步

- 后续再考虑设备选择、源格式/分辨率/FPS 选择、截图、录制和 loopback 输出。
- 对 GPUI 图像上传做更长时间 CPU/内存测量。

## 用户文件

- `background.pgm` 保持未跟踪、未修改，不加入提交。
- `Cargo.lock` 的既有未提交修改保持原样，不纳入本轮提交。
- `PROGRESS.md.lastround` 是本轮本地备份，保持忽略。
