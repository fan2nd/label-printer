# label-printer

这是 **Dioxus Desktop 原生桌面版**，不是旧的浏览器版。不用 Python，不需 Chrome/Edge 浏览器，也不启动 HTTP 服务。界面由操作系统的 WebView 渲染，CSV、模板、图像转换和设备通信都在本机 Rust 进程执行。

当前提供精臣 N1（USB 串口、BLE）和德佟 P2（BLE）两种打印驱动，均待本项目真机验收。设备连接、耗材匹配、点阵转换和逐张发送通过 `src/device.rs` 接入；预览、模板和批量任务共用同一套流程。

精臣 N1 本身支持 USB。使用精臣官方 PC 上位机时，N1 通过 USB 连接。本项目可选 USB 或蓝牙。USB 模式使用系统串口和公开协议，以 115200 波特率通信，收到型号 3586 的回复后启用 N1 打印。[精臣桌面端支持型号](https://www.niimbot.com/downloadCenter) · [第三方 USB 串口协议资料](https://printers.niim.blue/interfacing/connecting/#serial-usb)

## macOS 启动

在项目根目录运行 `cargo run --release --locked` 打开工作台。使用蓝牙连接时允许蓝牙访问。

## Windows x64

1. 解压 Windows 运行包，双击 `label-printer.exe`。请将 `WebView2Loader.dll` 与 exe 放在同一目录。
2. Windows 须有 [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。很多 Windows 10/11 已预装；它是界面运行时，不需要打开浏览器。
3. N1 使用 USB 时，打开打印机并接入 USB 数据线，在顶部选择「精臣 N1」→「USB」→ 打印机对应的端口，点击「连接 N1」。插线后可点「刷新端口」。蓝牙连接时，开启系统蓝牙和打印机，断开已占用设备的手机 App。在顶部选择「精臣 N1」或「德佟 P2」，点击连接。附近有多台同型号打印机时，先关闭其他设备。
4. N1 安装受支持的标签及对应色带。P2 将热敏间隙标签居中装入，选择对应纸张规格。用「试打当前张」核对方向、边距和条码，再运行批量打印。

系统适配：Windows 用系统 WinRT BLE，macOS 用 CoreBluetooth，Linux 用 BlueZ/D-Bus；USB 通过开源 Rust 库 serialport 访问系统串口。**Windows x64 文件在 Linux 交叉构建，未在 Windows 真机启动；N1 和 P2 均待本项目真机验收。**

## 操作

- N1 耗材编码 `05262201` 对应 12 × 25 mm 单张圆角亚银标签。25 mm 走纸方向首尾各留 1 mm，纸宽方向不扣减。每张仍发送 200 行，首尾各 8 行保留图案数据，可打区为 184 × 96 点（23 × 12 mm）。圆角半径尚未实测，预览用 1.5 mm 示意，打印仍保留边角点阵。
- N1 连接时会尝试读取耗材 RFID 编码。将实机读到并核实尺寸的编码填入对应 YAML 规格的 `barcodes` 字符串列表后，重新构建即可自动选用该规格。未命中时显示编码并保留当前规格。换卷后重新连接可刷新。[第三方 RFID 协议说明](https://github.com/iscarelli/niimbot-web-bluetooth/blob/main/docs/protocol-v4.md)
- P2 内置 48 × 50 mm 和 40 × 30 mm 规格。自定义规格最大纸宽 57 mm，203 dpi 下可打印宽度最多 384 点，可打印长度最多 1600 点。窄幅内容按居中装纸转换为完整的 48 字节点阵行。
- 项目根目录的 [labels.yaml](labels.yaml) 定义内置规格，每个机型的第一项是默认规格。每项填写整数毫米的 `width_mm`、`length_mm`，可选 `name`。Cargo 构建时校验并嵌入规格。每个方向按毫米 × 203 ÷ 25.4 四舍五入到整点，横向再取打印头宽度上限（N1 为 96 点，P2 为 384 点）。编辑 YAML 后重新构建即可更新下拉列表。自定义规格可预览或导出 SVG；在其他耗材没有验收前，N1 实际发送仅开放上述规格。切换规格不会静默缩放或裁切版面，已有元素超界会提示。
- YAML 规格的 `cut` 可记录 `rounded`（圆角）、`combined`（组合标签）和 `feed_margin_mm`（走纸首尾各留多少毫米）。如有实测刀模，可用 `radius_dots` 指定圆角半径，组合标签用 `regions_dots` 列出各区的 `[x, y, 宽, 高]`，坐标单位为打印点，x 沿走纸方向。预览用灰底提示走纸间隙，落在间隙中的墨点仍按黑色显示并发送。已配置的圆角和分区按刀模裁切。
- 工作台默认显示「打印预览」。N1 和 P2 均按 203 dpi 输出，预览使用对应机型的单色打印数据。可直接拖动选框调整位置，或点「放大查看」核对小字和细线。
- 「加 图片」支持 PNG、JPEG 和 BMP。导入时按比例适配标签，透明区域按白底处理。可拖动右下角或输入宽高调整尺寸，在「黑白阈值」「抖动」「反色」中调整取模效果。模板内保存灰度图和参数，重新打开仍可调整。
- 文字字号用打印点指定。增删字符、切换 CSV 记录时，选框按当前字形自动收紧或展开，字号保持固定。
- 二维码和 Code 128 的选框贴合码图。拖动右下角按整模块缩放；二维码保持正方形，条码的条高可以单独调整。方向键移动元素，Shift 加大步长，Alt + 方向键调整码图尺寸。
- 启动和「载入默认」均使用空白标签。「删除越界元素」按当前记录和耗材规格一次移除越界元素，包括墨点落在首尾留白或刀模间隙中的元素。
- 元素可拖出标签四边，也可直接输入负坐标。预览提示当前记录的越界元素和码图遮挡；试打和批量打印均保留走纸间隙内的图案，空白标签也可打印。布局时请在二维码四周、条码左右留出扫码所需的空白，并试打核对。
- `{seq}` 是递增序号，`{列名}` 绑定 CSV 字段。无 CSV 时按记录数生成，导入 CSV 后每行一条记录。每条记录可连续打印多份，各份使用相同序号，单批最多 500 张。可逐张预览、试打或导出当前 SVG，其他记录的排版错误在整批打印前检查。
- 支持从本地文件导入 UTF-8 CSV，含引号、逗号和 BOM。
- 使用「保存模板」加入本机列表，用「导出模板」得到可移交的版本化 JSON。新版导出 v2，可读取 v1 模板。文件保存介质和全部元素（保留 `{seq}`、CSV 列名占位符），不保存 CSV 数据、当前张数或蓝牙设备。另一台机器可用「导入模板」打开，若希望加入本机列表再点保存。导出 SVG 不是导出可编辑模板。
- 模板写入本机用户应用数据目录的 `templates.json`。已有 `media.json` 中的自定义规格和旧模板保存的可打印点数仍可使用。此版本不读取旧浏览器版的站点存储。
- 编辑时的排版警告用于即时修正；连接、读写、保存、打印等已执行操作失败时出现独立弹窗。右下角「确认」只关闭提示，不重试、不重发，不改变已有数据。
- 「试打当前张」只打印预览中的一张。批量打印期间可点「停止后续打印」，正在发送的当前张会完成本次传输。N1 显示设备确认张数，P2 显示已发送张数，实际出纸需核对实物。
- 发送前整批检查并生成点阵，逐张发送。失败后不会自动重发；按界面显示的进度核对实物再继续。浏览器版与桌面版字体栅格不同，打印中文建议系统安装相应字体（微软雅黑、苹方、Noto Sans CJK SC 等）。

## 从源码构建

桌面 UI 构建使用 Rust/Cargo，无需 Dioxus CLI 或 Python。随包构建以 Rust 1.100 为验收工具链。

```sh
cargo build --release --locked
cargo test --no-default-features --features ble --locked
```

- Windows 与 macOS：在目标机上安装 Rust 及目标平台的工具链。
- Debian/Ubuntu 系 Linux：构建 UI 需要 GTK 3、WebKitGTK 4.1、适用的系统托盘开发包和 pkg-config。BLE 需要运行中的 BlueZ、系统 D-Bus 和可用的蓝牙适配器。当前提供 Linux 源码。
- 窗口与样式由 Dioxus Desktop 0.7 运行；蓝牙由 `btleplug` 执行。打印协议参考 [niimbot-web-bluetooth 2.6.0](https://www.npmjs.com/package/niimbot-web-bluetooth/v/2.6.0)，按其公开 N1 规格与时序在 Rust 中实现，保留 `THIRD-PARTY-NIIMBOT.txt`。

## 设备准入与第三方协议资料

准入依据是打印 API 完全透明：上位机使用的命令格式、图像编码、传输过程以及成功状态的含义均可审阅。项目独立实现或使用授权明确的开源代码，打印链路无需厂商闭源 SDK。

| 型号 | 可核查的实现或实机记录 | 本项目状态 |
| --- | --- | --- |
| 精臣 N1 | 本项目使用 Rust 实现 USB 串口和 BLE 通信、图像旋转和点阵发送。[USB 驱动](src/printer/usb.rs) · [第三方串口实现](https://github.com/MultiMote/niimbluelib/blob/main/src/client/serial_impl.ts) · [设备入口](src/device.rs) · [协议实现](src/printer.rs) · [点阵转换](src/raster.rs) · [第三方协议参考](https://www.npmjs.com/package/niimbot-web-bluetooth/v/2.6.0) · [许可声明](THIRD-PARTY-NIIMBOT.txt) | 已实现上位机驱动，待本项目实机验收。 |
| 德佟 P1 / DT01 | 第三方实机抓包记录显示，打印使用经典蓝牙 RFCOMM 通道 1，BLE 可读取状态和计数器。其 Python 打印入口默认选择 `sdk-compact`，编码器会加载随附 SDK。可参考独立抓包记录和传输逻辑。[逆向记录](https://github.com/NicoSeg735/label-printer/blob/main/HISTORIAL_INVESTIGACION.md) · [传输源码](https://github.com/NicoSeg735/label-printer/blob/main/printer.py) | 仅作协议研究。无厂商 SDK 的完整编码与出纸链路待验证。 |
| 德佟 P2 | 第三方 Rust 项目报告在 P2 上测试了 BLE 打印。协议编码与传输分包实现，公开入口有 `scan`、`connect` 和 `PrinterConnection::print_png`。[项目与测试型号](https://github.com/IvanLi-CN/detonger) · [Rust API](https://github.com/IvanLi-CN/detonger/blob/3ef5904738bb528e8145ab3b9eb71bafc542341f/crates/detonger-printer/src/lib.rs) · [帧格式与点阵编码](https://github.com/IvanLi-CN/detonger/blob/3ef5904738bb528e8145ab3b9eb71bafc542341f/docs/protocol.md) | 本项目依据公开协议独立实现 BLE 驱动，安装时无需上游库。待本项目实机验收。[本项目 P2 驱动](src/p2.rs) · [BLE 服务与特征说明](https://github.com/IvanLi-CN/detonger/blob/3ef5904738bb528e8145ab3b9eb71bafc542341f/docs/ble.md) |

### 其他有公开协议的候选机型

- **德佟 DP27P（USB）**。`dz-print` 有本型号的实机记录及 MPL-2.0 驱动。P2 和 DP27P 都出现 `0x1F 0x2B` 点阵行命令。P2 的参考实现走 BLE，每行最多 384 点；DP27P 样机的打印宽度为 576 点，USB 还涉及 HID 分包、缓冲区流控和按版本计算的校验。两者的打印数据不能直接互换。[DP27P 逆向协议](https://github.com/sb-child/dz-print/blob/master/protocol.md) · [USB 流控](https://github.com/sb-child/dz-print/blob/master/flow-control.md) · [开源驱动](https://github.com/sb-child/dz-print)
- **Brother QL-800（USB）**。Brother 发布了具体到型号的点阵命令和状态回读规范，第三方 `brother_ql` 将 QL-800 标记为已验证，可对照 USB 实现。[点阵命令手册](https://download.brother.com/welcome/docp100278/cv_ql800_eng_raster_101.pdf) · [驱动与型号清单](https://github.com/pklaus/brother_ql)
- **Zebra ZD421**。该系列支持公开的 ZPL II 指令集，可独立生成打印数据。接口随具体配置变化，网络打印需要带网络接口的机型。[型号规格](https://www.zebra.com/content/dam/zebra_dam/en/tech-specs/zd421-tech-specs-en-us.pdf) · [ZPL 指令手册](https://www.zebra.com/content/dam/support-dam/en/documentation/unrestricted/guide/software/zpl-zbi2-pg-en.pdf) · [第三方 ZPL 生成器](https://github.com/cod3monk/zpl)

## 验证边界

测试覆盖媒体、CSV、序号、条码、96 × 200 点阵旋转、BLE 封包与行编码，以及画布移动/缩放边界和模板文件往返。P2 测试覆盖居中点阵转换与 BLE 消息编码。两种型号均需实机核对连接、走纸、批量和扫码结果。

Windows 包随附微软 x64 `WebView2Loader.dll`，按 [Microsoft 的桌面分发说明](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution) 部署；不捆绑 WebView2 Runtime 安装程序。
