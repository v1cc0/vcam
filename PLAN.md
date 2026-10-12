# vcam GPUI Kit 预览界面计划

## 目标

新增 `vcam --preview`，使用 `gpui-kit 0.7.1` 提供一个接近 AMCap2 的 Linux 桌面预览界面：设备/格式信息、中央实时画面、状态栏和后续控制入口。现有 `vcam run` 的 V4L2 loopback 与 OBS 行为保持不变。

## 已完成：第一轮预览壳

- 接入 `gpui-kit 0.7.1` 与 RGBA/BGRA 图像依赖，锁文件已更新。
- 新增 `src/preview.rs`，采集线程独占 V4L2 设备，GPUI 线程只消费最新帧。
- `vcam --preview [options]` 选择物理摄像头，支持现有 MJPEG/YUYV 输入并显示实时画面。
- 实现纯黑背景、纯白文字的 AMCap 风格工作区：顶部设备/格式信息、中央左右分屏预览（左侧原始帧、右侧处理帧）、左侧设备与 Camera controls、右侧软件效果控制和底部状态栏。
- 右侧 Software effects 接入亮度、对比度滑块和旋转/翻转下拉菜单，仅作用于右侧处理预览；原始帧保持不变。
- `Camera controls` 已移入左侧 Device/Capture/Output 信息下方，提供 `Reset camera defaults` 按钮，按 V4L2 驱动报告的默认值恢复可用硬件参数，并同步滑块和复选框状态。
- 中间预览列和图像元素设置明确的 flex 收缩边界，防止原始图像覆盖右侧面板。
- 原始/处理两套动态 GPUI 图像都在替换前显式回收旧 atlas 资源。
- 关闭窗口时通知采集线程退出；现有 `vcam run`、`list`、`snapshot`、`bench` 行为未改。

## 验证

- `cargo fmt --all -- --check` 通过。
- `cargo check` 通过。
- `cargo build --release` 通过。
- `cargo test`：22 个测试通过，包含 YUYV 预览颜色转换及下拉映射回归测试。
- `vcam --help` 能列出 `vcam --preview [options]`。
- 使用 `/dev/video1`、`640x480@15` 做短时 GUI smoke test，窗口事件循环保持运行 12 秒，无采集/窗口错误输出。

## 已完成：左右分屏与合并效果菜单

- 中央预览改为左右布局：左侧显示原始帧，右侧显示处理帧；旋转滤镜选择后可获得更大的横向观察区域。
- 左侧 Camera controls 保留全部 V4L2 控件和默认值恢复；右侧只保留 Software effects。
- Video filters 已并入 Software effects，以 Video rotation 和 Flip 两个下拉菜单提供 None、顺/逆时针、半转、水平/垂直翻转选项；亮度和对比度继续使用连续滑块。
- 下拉选择会同步相互排他的旋转/翻转状态，避免界面显示过期选项。


## 第二轮验证

- `cargo fmt --all -- --check`、`cargo check`、`cargo test` 和 `git diff --check` 通过。
- `cargo test`：22 个测试通过，新增 GPUI 滑块范围、反向 V4L2 控件边界和下拉索引映射回归测试。
- `/dev/video1` 的真实 Wayland smoke test（`640x480@15`）启动无 panic、无日志错误，采集帧数持续增长；niri 识别 App ID `com.v1cc0.vcam`。
- 截图确认左侧设备栏及 Camera controls、中间原始/处理左右预览、右侧 Software effects 互不覆盖；预览进程和临时文件已清理。
## 下一步

- 增加设备选择、源格式/分辨率/FPS 选择、截图、录制和输出到 loopback 的交互。
- 对 GPUI 图像上传的 CPU/内存成本做 30 秒测量；超过预算时再评估独立视频 surface 或低分辨率 UI 预览。
- 使用真实摄像头完成窗口截图和 30 秒稳定性验证。

## 约束与风险

- 使用最新帧覆盖的有界状态，宁可丢旧帧也不阻塞采集。
- 当前 GPUI 图像路径会产生 CPU 到 GPU 的帧上传，只作为第一阶段预览路径，不承诺零拷贝或 60 FPS。
- 不把第三方示例代码或依赖源码复制进仓库；只依赖 `gpui-kit` 公开 API。
- `background.pgm` 是用户未跟踪文件，必须保持原样且不加入提交。
