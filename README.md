# label-printer

这是 **Dioxus Desktop 原生桌面版**，不是旧的浏览器版。不用 Python，不需 Chrome/Edge 浏览器，也不启动 HTTP 服务。界面由操作系统的 WebView 渲染，Excel、CSV、模板、图像转换和设备通信都在本机 Rust 进程执行。

当前提供精臣 N1（USB 串口、BLE）和德佟 P2（USB HID、BLE）打印驱动。设备连接、耗材匹配、点阵转换和逐张发送通过 `src/device.rs` 接入；预览、模板和批量任务共用同一套流程。

精臣 N1 本身支持 USB。使用精臣官方 PC 上位机时，N1 通过 USB 连接。本项目可选 USB 或蓝牙。USB 模式使用系统串口和公开协议，以 115200 波特率通信，收到型号 3586 的回复后启用 N1 打印。[精臣桌面端支持型号](https://www.niimbot.com/downloadCenter) · [第三方 USB 串口协议资料](https://printers.niim.blue/interfacing/connecting/#serial-usb)

## macOS 启动

在项目根目录运行 `make mac-app`，然后在 Finder 双击 `target/release/label-printer.app` 打开工作台。旁边无扩展名的可执行文件供终端调用。使用蓝牙连接时允许蓝牙访问。

## 命令行

直接启动打开桌面工作台。传入子命令可在终端查看规格、生成点阵 PNG 或打印一张。以下示例使用 P2 与 40 × 30 mm 标签。

```sh
target/release/label-printer media
target/release/label-printer preview p2 /tmp/label.png --media 40x30 --text '物料 A-01'
target/release/label-printer print p2 usb --media 40x30 --text '物料 A-01'
target/release/label-printer print p2 ble --template label.json
```

`--text` 接受单行原文。桌面导出的模板可用 `--template` 读取，序号默认取 0001，当前命令只处理单张内容。N1 的 USB 端口可用 `ports` 查看，再传入 `--port`。运行 `--help` 查看连接方式和用法。失败信息写入标准错误并返回非零退出码。

桌面「复制为新模板」保留当前规格、元素和变量，并生成独立名称。修改副本后点「保存模板」即可复用。「新建空白」沿用当前耗材规格。

CLI 与桌面共用本机模板库。`templates` 列出名称，`--saved` 按名称载入。文件模板和本机模板都可重复使用 `--set 字段=值` 填写变量，用 `--seq` 设置本张序号。例如模板内容为「{名称} {seq}」时可运行：

```sh
target/release/label-printer templates
target/release/label-printer preview p2 /tmp/label.png --saved '物料标签' --set '名称=接线端子' --seq 0018
```

## Windows x64

1. 解压 Windows 运行包，双击 `label-printer.exe`。请将 `WebView2Loader.dll` 与 exe 放在同一目录。
2. Windows 须有 [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。很多 Windows 10/11 已预装；它是界面运行时，不需要打开浏览器。
3. N1 使用 USB 时，打开打印机并接入 USB 数据线，点顶部「连接打印机」，选择「精臣 N1」→「USB」→ 打印机对应的端口，点击「连接」。插线后可点「刷新端口」。蓝牙连接时，开启系统蓝牙和打印机，断开已占用设备的手机 App。点顶部「连接打印机」，选择「精臣 N1」或「德佟 P2」，点击连接。附近有多台同型号打印机时，先关闭其他设备。
4. N1 安装受支持的标签及对应色带。P2 将热敏间隙标签居中装入，选择对应纸张规格。用「试打当前页」核对方向、边距和条码，再运行批量打印。

系统适配：Windows 用系统 WinRT BLE，macOS 用 CoreBluetooth，Linux 用 BlueZ/D-Bus；N1 USB 访问系统串口，P2 USB 使用 hidapi 访问 HID 接口。**Windows x64 文件在 Linux 交叉构建，未在 Windows 真机启动；Windows 上的设备连接和打印仍待验证。**

## 标签规格方向公约

耗材规格统一写作 **纸宽 × 走纸长（mm）**。纸宽沿打印头，垂直于走纸方向，对应 `width_mm`。走纸长沿纸带，对应 `length_mm`。模切标签取单张外形尺寸。

编辑画布以左上角为原点。**X 沿走纸轴向右增加，Y 沿纸宽轴向下增加**。四边标定按实物出纸方向读图。箭头朝上，T 是先出纸边，B 是最后出纸边，L/R 是此时纸面的左/右边。点阵尺寸按 X × Y 书写。例如 P2 的 40 × 30 mm 标签，纸宽 40 mm、走纸长 30 mm，在 203 dpi 下画布为 240 × 320 点。

在编辑区点击「旋转视图」可顺时针转动 90°。拖动和方向键跟随屏幕方向，位置输入框仍使用标签的 X/Y 坐标。保存模板、导出和打印均使用标签坐标。选中元素后，可用「元素旋转」单独指定顺时针角度（0°、90°、180°、270°）。元素占用区域的左上角保持在原 X/Y，角度随模板保存，并用于预览和打印。

## 元器件模板

德佟 P0 的 12 × 30 mm 和 12 × 22 mm 圆角纸各有电阻、电容、MOS 管和三极管料盒模板。选好规格后在「模板」中点选即可。

精臣 N1 的 12 × 25 mm 圆角亚银标签（耗材编码 05262201）内置五款模板：电阻料盒、电容料盒、阻容分装、三极管料盒和 MOS 管料盒。在「模板」里点选后，可在「数据」修改示例值，也可导入 Excel 或 CSV 批量打印。每张两行。首行显示阻容值或型号，次行显示封装与识别所需的精度、耐压、批次、极性或沟道。内置布局和示例值见[模板库](assets/templates.json)。

```sh
target/release/label-printer preview n1 /tmp/resistor.png --saved '电阻料盒' --set '阻值=22 kΩ'
```

## 操作

文字和序号可在「属性 → 字体」选用本机安装的字体。模板记录字体名称，另一台电脑缺少该字体时使用默认字形。

左侧元素列表从前层到后层排列，拖动一行或使用上移、下移可调整叠放顺序。顺序随模板保存，并用于预览和打印。

从「插入组件」选择文字、图片、码图或序号，选中后在「属性」调整。元素工具可添加直线、矩形和椭圆。直线可调长度和线宽，矩形和椭圆可选空心或实心。选中任意元素可复制，副本使用独立 ID，并在 X、Y 方向各错开 4 点。

- N1 耗材编码 `05262201` 对应 12 × 25 mm 单张圆角亚银标签。整张点阵为 200 × 96 点。圆角半径尚未实测，预览用 1.5 mm 示意，打印仍保留边角点阵。
- 在「规格 → 标定四边范围」可预览并试打一张 [N1 四边标定标签](assets/n1-feed-range.svg)。测试发送完整 200 × 96 点，跳过走纸校正和刀模裁切。每端从 0 向内计数，长刻度每 8 点，数字每 16 点。左右尺的阶梯从上到下、上下尺的阶梯从左到右依次为 0～7。例如最外侧可见台阶位于 8 点组的第三级，该边缺 10 点（约 1.25 mm）。按纸宽 × 走纸长计算，有效范围为 (96 − L − R) × (200 − T − B) 点。纸面物理留白用尺量各纸边到对应标尺线端点的距离。
- N1 连接时会尝试读取耗材 RFID 编码。将实机读到并核实尺寸的编码填入对应 YAML 规格的 `barcodes` 字符串列表后，重新构建即可自动选用该规格。未命中时显示编码并保留当前规格。换卷后重新连接可刷新。[第三方 RFID 协议说明](https://github.com/iscarelli/niimbot-web-bluetooth/blob/main/docs/protocol-v4.md)
- P2 可选蓝牙或 USB。USB 自动识别 HID 设备，通过命令查询实际型号；部分系统将它显示为 “DP27 Label Printer”。已在同一台 P2 的 USB 和 BLE 上分别发送一张 40 × 30 mm 四边标定图，设备印张计数各增 1。选该规格后，在「规格 → 标定四边范围」可重打。图上的实线位于纸面坐标边缘，虚线内缩 8 点（约 1 mm）。[HID 封装](https://github.com/sb-child/dz-print/blob/8792cac50749c29273ee6fe05ea02af5bbe0abb9/protocol.md) · [USB 缓冲区流控](https://github.com/sb-child/dz-print/blob/8792cac50749c29273ee6fe05ea02af5bbe0abb9/flow-control.md)
- P2 默认使用 40 × 30 mm 规格。自定义规格最大纸宽 57 mm，203 dpi 下可打印宽度最多 384 点，可打印长度最多 1600 点。窄幅内容按居中装纸转换为完整的 48 字节点阵行。
- [标签类型库](assets/labels.yaml) 定义内置规格，每个机型的第一项是默认规格。每项填写整数毫米的 `width_mm`、`length_mm`，可选 `name`。Cargo 构建时校验并嵌入规格。每个方向按毫米 × 机型分辨率 ÷ 25.4 四舍五入到整点，纸宽方向再取打印头宽度上限（N1 为 96 点，P2 为 384 点）。在「规格」选择标签。自定义规格可预览；在其他耗材没有验收前，N1 实际发送仅开放上述规格。切换规格不会静默缩放或裁切版面，已有元素超界会提示。
- YAML 规格的 `cut` 可记录 `rounded`（圆角）和 `combined`（组合标签）。如有实测刀模，可用 `radius_dots` 指定圆角半径，组合标签用 `regions_dots` 列出各区的 `[x, y, 宽, 高]`，坐标单位为打印点，x 沿走纸方向。已配置的圆角和分区按刀模裁切。
- 工作台默认显示「打印预览」。N1 和 P2 均按 203 dpi 输出，预览使用对应机型的单色打印数据。可直接拖动选框调整位置。
- 「加 图片」支持 PNG、JPEG 和 BMP。导入时按比例适配标签，透明区域按白底处理。可拖动右下角或输入宽高调整尺寸，在「黑白阈值」「抖动」「反色」中调整取模效果。模板内保存灰度图和参数，重新打开仍可调整。
- 文字字号用打印点指定。增删字符、切换表格记录时，选框按当前字形自动收紧或展开，字号保持固定。
- 二维码和 Code 128 的选框贴合码图。拖动右下角按整模块缩放；二维码保持正方形，条码的条高可以单独调整。方向键移动元素，Shift 加大步长，Alt + 方向键调整码图尺寸。
- 启动和「载入默认」均使用空白标签。「删除越界元素」按当前记录和耗材规格一次移除越界元素，包括墨点落在首尾留白或刀模间隙中的元素。
- 元素可拖出标签四边，也可直接输入负坐标。预览提示当前记录的越界元素和码图遮挡；试打和批量打印均保留走纸间隙内的图案，空白标签也可打印。布局时请在二维码四周、条码左右留出扫码所需的空白，并试打核对。
- `{seq}` 是递增序号，`{字段名}` 引用模板数据。手动填写时按记录数生成，导入表格后每行一条记录。每条记录可连续打印多份，各份使用相同序号，单批最多 500 张。可逐张预览和试打，其他记录的排版错误在整批打印前检查。
- 在「数据 → 导入表格」选择 Excel（.xlsx、.xls、.xlsm）或 UTF-8 CSV，也可以直接粘贴 CSV。Excel 取工作表第一行有内容的行作表头，跳过空行，多工作表可切换。
- 表头优先按字段名匹配，再识别常见写法，如 MPN 对应型号、Footprint 对应封装。只有缺列或有歧义的字段需要指定列，也可给整批填写固定值。没有引用的列不会印到标签上。
- 使用「保存模板」加入本机列表，用模板面板的「导出」得到可移交的版本化 JSON。新版导出 v2，可读取 v1 模板。文件保存介质和全部元素（保留 `{seq}`、表格字段占位符），不保存导入的表格数据、当前张数或蓝牙设备。另一台机器可用「导入模板」打开，若希望加入本机列表再点保存。
- 模板写入本机用户应用数据目录的 `templates.json`。模板保存纸张尺寸与可打印点数。此版本不读取旧浏览器版的站点存储。
- 编辑时的排版警告用于即时修正；连接、读写、保存、打印等已执行操作失败时出现独立弹窗。右下角「确认」只关闭提示，不重试、不重发，不改变已有数据。
- N1 和 P2 蓝牙意外断开时会弹窗提示，并展开连接面板。批量打印会停止发送后续标签，并显示已确认或已发送的张数。
- P2 可在顶栏设备按钮旁选择打印浓度（1～15，默认 1）。USB 和蓝牙共用该设置，命令行打印可用 `--density 10` 指定。
- 「试打当前页」只打印预览中的一张。批量打印期间可点「停止后续打印」，正在发送的当前张会完成本次传输。N1 显示设备确认张数，P2 显示已发送张数，实际出纸需核对实物。
- 发送前整批检查并生成点阵，逐张发送。失败后不会自动重发；按界面显示的进度核对实物再继续。浏览器版与桌面版字体栅格不同，打印中文建议系统安装相应字体（微软雅黑、苹方、Noto Sans CJK SC 等）。

## 从源码构建

桌面 UI 构建使用 Rust/Cargo，无需 Dioxus CLI 或 Python。随包构建以 Rust 1.100 为验收工具链。

```sh
cargo build --release --locked
cargo test --no-default-features --features ble --locked
```

- Windows 与 macOS：在目标机上安装 Rust 及目标平台的工具链。
- Debian/Ubuntu 系 Linux：构建 UI 需要 GTK 3、WebKitGTK 4.1、适用的系统托盘开发包、pkg-config 和 libudev-dev。BLE 需要运行中的 BlueZ、系统 D-Bus 和可用的蓝牙适配器。当前提供 Linux 源码。
- 窗口与样式由 Dioxus Desktop 0.7 运行；蓝牙由 `btleplug` 执行。打印协议参考 [niimbot-web-bluetooth 2.6.0](https://www.npmjs.com/package/niimbot-web-bluetooth/v/2.6.0)，按其公开 N1 规格与时序在 Rust 中实现，保留 `THIRD-PARTY-NIIMBOT.txt`。

## 设备准入与第三方协议资料

准入依据是打印 API 完全透明：上位机使用的命令格式、图像编码、传输过程以及成功状态的含义均可审阅。项目独立实现或使用授权明确的开源代码，打印链路无需厂商闭源 SDK。

| 型号 | 可核查的实现或实机记录 | 本项目状态 |
| --- | --- | --- |
| 精臣 N1 | 本项目使用 Rust 实现 USB 串口和 BLE 通信、图像旋转和点阵发送。[USB 驱动](src/printer/usb.rs) · [第三方串口实现](https://github.com/MultiMote/niimbluelib/blob/main/src/client/serial_impl.ts) · [设备入口](src/device.rs) · [协议实现](src/printer.rs) · [点阵转换](src/raster.rs) · [第三方协议参考](https://www.npmjs.com/package/niimbot-web-bluetooth/v/2.6.0) · [许可声明](THIRD-PARTY-NIIMBOT.txt) | 已实现上位机驱动，待本项目实机验收。 |
| 德佟 P0 | 已查阅的第三方项目分别记录了 P2 蓝牙打印和 DP27P USB 打印，尚缺 P0 的点阵格式、传输与出纸记录。[P2 协议项目](https://github.com/IvanLi-CN/detonger) · [DP27P 驱动](https://github.com/sb-child/dz-print) | 内置 12 × 30 mm、12 × 22 mm 单张圆角规格，可编辑和预览。打印协议待验证。 |
| 德佟 P1 / DT01 | 第三方实机抓包记录显示，打印使用经典蓝牙 RFCOMM 通道 1，BLE 可读取状态和计数器。其 Python 打印入口默认选择 `sdk-compact`，编码器会加载随附 SDK。可参考独立抓包记录和传输逻辑。[逆向记录](https://github.com/NicoSeg735/label-printer/blob/main/HISTORIAL_INVESTIGACION.md) · [传输源码](https://github.com/NicoSeg735/label-printer/blob/main/printer.py) | 仅作协议研究。无厂商 SDK 的完整编码与出纸链路待验证。 |
| 德佟 P2 | 第三方 Rust 项目报告在 P2 上测试了 BLE 打印。协议编码与传输分包实现，公开入口有 `scan`、`connect` 和 `PrinterConnection::print_png`。[项目与测试型号](https://github.com/IvanLi-CN/detonger) · [Rust API](https://github.com/IvanLi-CN/detonger/blob/3ef5904738bb528e8145ab3b9eb71bafc542341f/crates/detonger-printer/src/lib.rs) · [帧格式与点阵编码](https://github.com/IvanLi-CN/detonger/blob/3ef5904738bb528e8145ab3b9eb71bafc542341f/docs/protocol.md) | 本项目依据公开协议独立实现 BLE 驱动，安装时无需上游库。除第三方记录的 49535343 系列外，实机还发现 FF00 服务下的 FF02 可写特征。该通道已用于实机标定试打。[本项目 P2 驱动](src/p2.rs) · [BLE 服务与特征说明](https://github.com/IvanLi-CN/detonger/blob/3ef5904738bb528e8145ab3b9eb71bafc542341f/docs/ble.md) |

### 其他有公开协议的候选机型

- **德佟 DP27P（USB）**。`dz-print` 有本型号的实机记录及 MPL-2.0 驱动。P2 和 DP27P 都出现 `0x1F 0x2B` 点阵行命令。P2 的参考实现走 BLE，每行最多 384 点；DP27P 样机的打印宽度为 576 点，USB 还涉及 HID 分包、缓冲区流控和按版本计算的校验。两者的打印数据不能直接互换。[DP27P 逆向协议](https://github.com/sb-child/dz-print/blob/master/protocol.md) · [USB 流控](https://github.com/sb-child/dz-print/blob/master/flow-control.md) · [开源驱动](https://github.com/sb-child/dz-print)
- **Zebra ZD421**。该系列支持公开的 ZPL II 指令集，可独立生成打印数据。接口随具体配置变化，网络打印需要带网络接口的机型。[型号规格](https://www.zebra.com/content/dam/zebra_dam/en/tech-specs/zd421-tech-specs-en-us.pdf) · [ZPL 指令手册](https://www.zebra.com/content/dam/support-dam/en/documentation/unrestricted/guide/software/zpl-zbi2-pg-en.pdf) · [第三方 ZPL 生成器](https://github.com/cod3monk/zpl)

## 验证边界

测试覆盖媒体、Excel、CSV、列匹配、图层排序、序号、条码、96 × 200 点阵旋转、BLE 封包与行编码，以及画布移动/缩放边界和模板文件往返。P2 测试覆盖居中点阵转换与 BLE 消息编码。

Windows 包随附微软 x64 `WebView2Loader.dll`，按 [Microsoft 的桌面分发说明](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution) 部署；不捆绑 WebView2 Runtime 安装程序。
