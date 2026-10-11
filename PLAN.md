# vcam GPUI Kit 预览界面计划

## 目标

新增 `vcam --preview`，使用 `gpui-kit 0.7.1` 提供一个接近 AMCap2 的 Linux 桌面预览界面：设备/格式信息、中央实时画面、状态栏和后续控制入口。现有 `vcam run` 的 V4L2 loopback 与 OBS 行为保持不变。

## 已完成：第一轮预览壳

- 接入 `gpui-kit 0.7.1` 与 RGBA/BGRA 图像依赖，锁文件已更新。
- 新增 `src/preview.rs`，采集线程独占 V4L2 设备，GPUI 线程只消费最新帧。
- `vcam --preview [options]` 选择物理摄像头，支持现有 MJPEG/YUYV 输入并显示实时画面。
- 实现深色 AMCap 风格工作区：顶部设备/格式信息、中央 16:9 预览、左右信息/控制面板、底部状态栏。
- 关闭窗口时通知采集线程退出；现有 `vcam run`、`list`、`snapshot`、`bench` 行为未改。

## 验证

- `cargo fmt --all -- --check` 通过。
- `cargo check` 通过。
- `cargo build --release` 通过。
- `cargo test`：18 个测试通过，包含 YUYV 预览颜色转换回归测试。
- `vcam --help` 能列出 `vcam --preview [options]`。
- 使用 `/dev/video1`、`640x480@15` 做短时 GUI smoke test，窗口事件循环保持运行 12 秒，无采集/窗口错误输出。

## 下一步

- 从 V4L2 枚举并绑定曝光、增益、亮度、对比度、饱和度等真实控制项。
- 增加设备选择、源格式/分辨率/FPS 选择、截图、录制和输出到 loopback 的交互。
- 对 GPUI 图像上传的 CPU/内存成本做 30 秒测量；超过预算时再评估独立视频 surface 或低分辨率 UI 预览。
- 使用真实摄像头完成窗口截图和 30 秒稳定性验证。

## 约束与风险

- 使用最新帧覆盖的有界状态，宁可丢旧帧也不阻塞采集。
- 当前 GPUI 图像路径会产生 CPU 到 GPU 的帧上传，只作为第一阶段预览路径，不承诺零拷贝或 60 FPS。
- 不把第三方示例代码或依赖源码复制进仓库；只依赖 `gpui-kit` 公开 API。
- `background.pgm` 是用户未跟踪文件，必须保持原样且不加入提交。
