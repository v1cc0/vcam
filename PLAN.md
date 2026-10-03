# vcam 实时画面效果实施计划

## 目标

为夜间卧室里的 Vibe Coding 直播，基于当前 `vcam` 做出低成本的《THE FIRST TAKE》式画面：高调、纯净、通透、低反差，人物边缘自然，不把卧室杂物和抠像痕迹暴露给 OBS。

核心技术方向是 Rust + `fearless_simd` 的安全 SIMD CPU 管线。现有 V4L2 loopback 路线先保留，OBS 仍然通过 **Video Capture Device** 使用 `/dev/video20` 等输出节点。

## 已确认的仓库基线

- 当前仓库是 Rust 二进制程序，不是 OBS 原生插件。
- 输入：V4L2 USB 摄像头，支持 MJPEG 和 YUYV。
- 输出：`v4l2loopback` 的 YUYV 视频节点。
- 当前处理顺序：解码 -> 全画面纵向 squeeze -> 3x3 luma 平滑 -> 旋转/翻转 -> 输出。
- 当前 `beautify_yuyv` 每帧复制源缓冲区，且处理是逐像素标量循环。
- 当前 `--compare`、`--no-preview`、帧率和尺寸参数可作为验证工具继续使用。
- 目前只有 `jpeg-decoder` 和 `v4l` 两个直接依赖，尚未引入 SIMD、分割模型或 OBS FFI。

## 视觉目标与非目标

### 第一版应该解决

1. 纯白或浅灰背景合成。
2. 人像 mask 边缘抗锯齿、羽化和轻量 Light Wrap，避免贴纸感。
3. 低反差、高光压缩、阴影提亮和轻微去饱和。
4. 适度肤色平滑，保护眼睛、头发和衣物细节。
5. 在 1280x720@30fps 下稳定进入 OBS，默认效果自然而不是明显的塑料磨皮。

### 暂不解决

- 物理光线无法凭空生成。屏幕光或顶灯造成的硬阴影、严重噪点和错误白平衡仍需要靠白墙反射光、窗纱或其他低成本实物改善。
- 第一阶段不做原生 OBS UI、GPU shader、4K 保证或完整化妆特效。
- 不立即删除现有 `--strength` 和 `--slim`，避免破坏已有命令行使用方式。新的面部塑形参数应独立命名。

## 总体架构

```text
V4L2 capture
    -> MJPEG decode / YUYV normalize
    -> reusable frame + scratch buffers
    -> optional segmentation mask
    -> mask cleanup: morphology + feather
    -> SIMD compositing: background + light wrap
    -> SIMD tone/color pass
    -> optional face-aware retouching
    -> existing rotation/flip
    -> YUYV v4l2loopback
    -> OBS Video Capture Device
```

### 接入策略

先不把当前程序改成 OBS `cdylib`。先在现有 V4L2 bridge 中验证帧格式、效果质量和性能，因为它已经能真实进入 OBS，调试成本最低。

稳定后再将纯图像处理部分抽成最小共享核心，届时可以选择：

- 继续使用 CLI bridge，适合当前 Linux/`v4l2loopback` 部署。
- 增加 `cdylib` OBS filter，复用同一个核心；该阶段才引入 `libobs` C API、FFI 生命周期和 OBS 的 RGBA/BGRA/texture 路线。

两条输出路线不应同时作为第一阶段目标。OBS 原生插件会额外带来 ABI、线程、图形后端和插件安装问题，不能用来替代当前可运行的端到端基线。

## SIMD 设计

### 使用方式

引入 `fearless_simd = "1.0"`；需要 `#[simd]` 时再引入其独立的 `fearless_simd_macros` companion crate。使用 `Level::new()`、`dispatch!` 和 `Simd` generic kernel，让 x86 在运行时选择可用的 SSE2/SSE4.2/AVX2/AVX-512 路径，并保留 scalar fallback。

不直接照抄对话中的 `std::simd::u8x16` 示例：那是表达思路的伪代码，不是 `fearless_simd` 1.0 的实际 API。第一步必须用仓库锁定版本编写一个最小 kernel spike，确认 load/store、lane 转换、mask 和尾部处理。

### 算子优先级

1. **逐行 luma kernel**：先替换现有 3x3 luma 平滑的内部像素循环；边界用标量处理，保证行为清楚。
2. **mask 混合 kernel**：对前景、mask、背景颜色执行无溢出的 u16/u32 定点混合；不要在 8-bit lane 中直接相乘。
3. **tone mapping kernel**：黑位抬升、阴影提亮、高光压缩、对比度和饱和度调整。需要浮点时再使用 SIMD float，不为每个效果都强制走 float。
4. **mask morphology/blur**：先用可分离 1D box blur 或小半径近似；再根据视觉结果决定是否需要高斯近似。边缘处理必须有明确的左右/上下边界规则。
5. **Light Wrap kernel**：用原始 mask 与羽化/收缩后的 mask 差值获得边缘权重，将浅色背景以很小强度混入边缘，不直接给人物整体加白色光晕。

### 数据布局决策

先基于当前 YUYV 做两个小 benchmark：

- 直接在交错 YUYV 中 SIMD 处理 Y，并保留 U/V。
- 将帧转换为可复用的 Y、U、V 或 RGBA scratch layout，再执行多个效果，最后转回 YUYV。

以实际 720p30 的端到端耗时和内存带宽决定，不凭直觉承诺“零拷贝”。当前主要效果集中在 luma 和 mask，若交错布局已经足够快，就不增加全帧 planar 转换。

所有缓冲区在分辨率变化时一次性调整；处理循环内不允许 `Vec` 增长或每帧 `to_vec()`。SIMD kernel 必须与 scalar reference 使用同一组输入样本做差分测试。

## 分阶段交付

### M0：基线和可测量性

- 保留现有 CLI 行为。
- 为 capture、JPEG decode、每个处理阶段、output wait 增加可选耗时统计；默认关闭。
- 记录分辨率、帧率、处理耗时 p50/p95、丢帧/超时和输出阻塞。
- 建立合成 YUYV 测试帧，不依赖摄像头才能运行单元测试和 benchmark。
- 记录当前标量实现的质量与性能，作为 SIMD 对照。

完成条件：能在不启动 OBS 的情况下重复测量同一帧；真实设备运行 60 秒没有新增错误。

### M1：处理核心整理和零分配

- 将帧布局、scratch buffer 和现有效果从 `main.rs` 中逐步移到处理模块；不先设计多实现 trait。
- 把 `squeeze_yuyv` 和 `beautify_yuyv` 的每帧复制改成初始化时分配的 scratch buffer。
- 保留 scalar kernel，作为正确性 oracle。
- 修正现有全画面 `--slim` 的产品含义：不把它伪装成面部塑形；后续面部塑形使用独立参数。

完成条件：固定分辨率的 steady-state frame loop 无堆分配，原有测试和 OBS 输入输出行为不变。

### M2：fearless SIMD spike 和首批 kernel

- 加入 `fearless_simd`，先实现 luma blend/blur 的最小 kernel。
- 用 `dispatch!` 覆盖 scalar fallback 和本机最佳 SIMD level。
- 处理尾部 lane、奇数行边界、YUYV 的偶数像素配对和 u8 饱和转换。
- 用 scalar oracle 做随机帧、全黑、全白、单边缘、渐变和噪声帧的比较。
- 对每个 kernel 做 release benchmark，不把单个 kernel 的理论微秒数当成整帧性能。

完成条件：SIMD 与 scalar 输出在定义的误差范围内一致；720p30 的总处理 p95 明显低于 33.3ms，并且没有为了 SIMD 增加帧丢失。

### M3：可用的高调画面效果

按以下顺序加入参数，避免一次增加过多旋钮：

- `preset natural|first-take|clean`，先提供固定且经过验证的默认值。
- 背景颜色：白色/浅灰色，预留亮度控制，避免白底溢出。
- 黑位、阴影、高光、对比度、饱和度和轻微暖色调。
- 轻量 luma 去噪与非常克制的锐化，锐化不能放大夜间噪声。
- 可选 `--stats`，方便直播前确认机器仍在预算内。

效果顺序固定为：mask/背景合成 -> 低反差 tone -> 肤色和整体色彩 -> 轻锐化 -> 几何变换。每个效果用 synthetic frame 测试极值和不变条件。

完成条件：在夜间摄像头素材上，人脸不发灰、白底不过曝、头发边缘不明显闪烁；原始画面与处理画面可以通过 `--compare` 复核。

### M4：mask、羽化和 Light Wrap

- 第一个版本允许使用外部或测试 mask，以先验证 compositing 和边缘算法。
- mask 统一为单通道低分辨率/全分辨率 buffer，明确 stride 和时间戳。
- 对 mask 做小半径 morphology、羽化和时间平滑；不要让一帧错误分割直接产生明显白边。
- 用边缘带权重做 Light Wrap；只作用于边缘，强度有上限，默认关闭或很低。
- 再评估人像分割运行时。模型推理作为独立可选模块，不能让基础 bridge 因模型文件、GPU runtime 或动态库缺失而无法启动。

完成条件：合成边缘在白墙、深色头发、耳机和麦克风轮廓上稳定；没有明显 halo、抖动或人物内部被误抹白。

### M5：面部感知和直播可靠性

- 在有人脸框/landmark 可靠后，才实现面部区域 retouching 和 face-slim；不再对整幅画面做纵向 squeeze。
- 对脸部区域使用 feathered mask，保护眼睛、眉毛、嘴唇、发际线和麦克风。
- 增加摄像头拔插恢复、设备重新选择和输出设备错误信息；保留显式 `--source` 优先级。
- 验证 OBS 长时间运行、关闭预览、无 OBS reader、输出缓冲阻塞和设备重连。
- 只有核心稳定后，才评估将处理核心复用成 OBS 原生 `cdylib` filter。

## 性能和质量验收

### 硬指标

- 首个目标：1280x720@30fps；后续再测 1920x1080@30fps。
- 30fps 单帧预算为 33.3ms，处理阶段目标 p95 <= 8ms，完整 capture-to-output 延迟目标 <= 20ms；最终数值以实机测量为准。
- steady-state 不发生逐帧堆分配。
- SIMD、scalar 和 fallback 都能通过同一组测试。
- YUYV 输出尺寸、偶数宽度、bytesused 和 OBS 协商保持正确。

对话中提到的“1080p <1ms”只能作为待验证的理想目标，不能写成承诺。JPEG 解码、mask 推理、内存拷贝和 V4L2 阻塞都不属于单个 SIMD kernel 的耗时。

### 视觉检查

- 原始摄像头、处理后画面并排检查。
- 白色/浅灰背景不过曝，皮肤不变成塑料或灰白。
- 头发、耳机、麦克风金属网边缘没有明显白边和闪烁。
- 阴影保持柔和但仍有立体感，不用软件抹平物理光线造成的硬阴影。
- 夜间噪声不因锐化、Light Wrap 或 mask 放大。

## 风险与决策门

- **`fearless_simd` API 变化**：锁定 Cargo 版本；先写小 spike，不把伪代码直接放进生产管线。
- **SIMD 不是主要瓶颈**：若 JPEG 解码或 mask 推理占大头，先优化帧调度和缓冲复用，不继续堆 kernel。
- **mask 质量决定观感**：错误分割比标量处理慢更容易毁掉画面，因此 mask 先独立验证，再接入直播默认 preset。
- **CPU 与 GPU 路线冲突**：第一阶段坚持 CPU SIMD；如果 OBS 原生插件阶段发现纹理上传/下载成为瓶颈，再单独评估 GPU shader，不在当前计划中混入两套渲染后端。
- **多线程可能增加延迟**：先单线程 SIMD 达到预算；只有 benchmark 证明 1080p/60 不够，再按行批处理并测量 Rayon 或线程池的调度成本。
- **物理条件不可替代**：软件计划不替代白墙反射光、窗纱、摄像头曝光锁定和镜头距离；这些是达到通透画面的必要输入。

## 下一步

从 M0 开始：先增加可选 stage timing 和固定 synthetic frame benchmark，再引入 `fearless_simd`。在 M0/M1 的性能基线没有数据之前，不接入 ONNX、`libobs` FFI 或 Rayon。
