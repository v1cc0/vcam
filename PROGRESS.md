# Progress

## 本轮完成

- 在 `vcam --preview` 的 `Camera controls` 面板加入 `Reset camera defaults` 按钮。
- 保存 V4L2 控件描述中的默认值；点击按钮时按 V4L2 控件顺序恢复所有可用参数，并同步硬件写回、滑块和复选框状态。
- 对当前 inactive 或写回失败的控件显示计数，不将部分成功伪装为全部成功。
- 配置 GPUI Kit 应用级主题：纯黑背景、纯白文字、纯白边框；Accordion、Slider、Checkbox、滚动条和滤镜选中态均不再回退到浅色默认样式。
- 更新 README 和 PLAN，保留 `vcam run`、实时双画面、控件写回和图像资源回收行为不变。

## 验证

- `cargo fmt --all -- --check` 通过。
- `cargo check` 通过。
- `cargo test`：21 项全部通过。
- `cargo build --release` 通过。
- `git diff --check` 通过。
- 已扫描 `src/preview.rs` 颜色值，显式颜色仅使用 `rgb(0x000000)` 和 `rgb(0xffffff)`；GPUI 组件主题同步使用黑白 HSL。
- `/dev/video1`、`640x480@15` 的 release Wayland smoke test 运行稳定，无日志错误；截图确认重置按钮可见、黑白主题生效、三栏布局和上下双预览无覆盖。
- niri 识别窗口 App ID：`com.v1cc0.vcam`。

## 下一步

- 增加设备选择、源格式/分辨率/FPS 选择、截图、录制和输出到 loopback 的交互。
- 对 GPUI 图像上传做更长时间 CPU/内存测量；此前的旧图像 atlas 累积已由 `Window::drop_image` 修复。

## 用户文件

- `background.pgm` 保持未跟踪、未修改，不加入提交。
- `PROGRESS.md.lastround` 是本轮本地备份，保持忽略。
