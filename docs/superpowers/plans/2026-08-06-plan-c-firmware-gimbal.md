# Plan C — 固件云台 MCP 工具 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `esp32-s3n16r8-emoji` 板上注册三个云台 MCP 工具（`self.gimbal.look` / `self.gimbal.set` / `self.gimbal.get_position`），让 nomifun 侧的伙伴能通过 `tools/call` 直接转动机器人的头。

**Architecture:** 板级新增一个头文件 `gimbal_mcp_tools.h`，内含一个常驻 worker 任务 + 一条"绝对目标角度"队列：MCP 工具回调只做参数解析、限位 clamp 与入队（O(1) 立即返回），真正耗时的 `ServoController::HeadMove` 步进循环在 worker 任务里串行执行。三个工具在 `EmojiBoard` 构造期（现有空实现 `InitializeIot()` 内）注册，通用层（`mcp_server.*` / `protocols/*` / `application.cc`）零改动。

**Tech Stack:** ESP-IDF / C++ / ESP32-S3 / LEDC PWM / MCP over WebSocket

## Global Constraints

1. **固件仓库不是 git 仓库**（`/home/developer/src/xiaozhi/xiaozhi-yuntai` 下无 `.git`）。本计划**任何步骤都不得使用 git 命令**（no `git status` / `git diff` / `git add` / `git commit` / `git stash`）。每个任务的收尾是**编译验证**，不是提交。
2. **所有改动限定在 `main/boards/esp32-s3n16r8-emoji/` 内。** 严禁改动 `main/mcp_server.h` / `main/mcp_server.cc` / `main/protocols/*` / `main/application.cc` / `main/display/*` / `main/CMakeLists.txt` / `sdkconfig`。本计划只新增 1 个头文件 + 修改 `emoji_board.cc` 4 处。
3. **工具名是跨仓库契约，逐字写死，不得改名、不得增删**：`self.gimbal.look`、`self.gimbal.set`、`self.gimbal.get_position`。参数名同样是契约：`direction`、`pan`、`tilt`。nomifun 侧 MCP 桥按这些名字对接。
4. **限位值以 `main/boards/esp32-s3n16r8-emoji/board_config.h:36-41` 的宏为唯一真相**：`SERVO_MIN_X`(=50) / `SERVO_MAX_X`(=130) / `SERVO_MIN_Y`(=70) / `SERVO_MAX_Y`(=110) / `SERVO_CENTER_X`(=90) / `SERVO_CENTER_Y`(=90)。新代码里**不得出现 50/130/70/110 这些字面量**，包括工具描述文本（描述在运行期用 `std::to_string(宏)` 拼出来，保证限位一改描述就跟着改）。
5. **不新增 `.cc` 文件。** `main/CMakeLists.txt:227-231` 用 `file(GLOB BOARD_SOURCES .../boards/${BOARD_TYPE}/*.cc)` 收集板级源码，**没有 `CONFIGURE_DEPENDS`**——新增 `.cc` 不会触发 CMake 重新配置，会静默不进构建。新增 `.h` 无此问题：板目录本身不在 `INCLUDE_DIRS` 里（`main/CMakeLists.txt:29`），但同目录的 `#include "xxx.h"` 靠"引用文件所在目录优先"解析，板内既有文件（`emoji_board.cc` 引 `board_config.h`）就是这么做的。
6. 计划正文中文；代码、标识符、日志 tag、MCP 工具描述文本一律英文（通用层工具描述也是英文；中文在 8000 字节的 `tools/list` 预算里每字 3 字节，无谓浪费）。
7. **禁止跑 `idf.py set-target`**：它会把现有 `sdkconfig` 挪成 `sdkconfig.old` 并按 defaults 重生，直接丢掉 `CONFIG_BOARD_TYPE_ESP32_S3N16R8_EMOJI=y`（`sdkconfig:700`）。`sdkconfig` 里已有 `CONFIG_IDF_TARGET="esp32s3"`，`idf.py build` 自己就能拿到目标。

## 为什么这个计划没有"先写失败测试"步骤

ESP-IDF 固件侧本仓库**没有任何单元测试框架**（无 Unity/host test/`test/` 目录），云台效果是物理量（舵机转角），不存在可断言的进程内被测对象。所以**不要按 TDD 硬套**。替代做法（每个任务固定三步）：

- **Step 1: 明确验证标准** —— 先写清"改完之后应该能观察到什么"，具体到可判定（编译产物、串口日志的具体行、`tools/list` 的具体内容、舵机的具体动作）。
- **Step 2: 实现** —— 抄计划里的完整代码块。
- **Step 3: 编译验证** —— 按下面「编译验证的两条路径」执行，并对照 Step 1 的标准判定。

## 编译验证的两条路径

任务 Step 3 里提到"编译验证"时，按下面二选一执行。

### 路径 A：ESP-IDF 环境可用（首选）

判定环境是否可用：

```bash
# 三条都要看：有 export.sh、能 source、source 后有 idf.py
ls "$HOME/esp/esp-idf/export.sh" 2>/dev/null || ls /opt/esp-idf/export.sh 2>/dev/null
echo "IDF_PATH=$IDF_PATH"
command -v idf.py
```

可用时这样构建：

```bash
# 1) 激活环境（路径按实际安装位置改；build/project_description.json 记录上次构建用的是 v5.5.3）
. "$HOME/esp/esp-idf/export.sh"

# 2) 现有 build/ 是在 Windows 上生成的（build/CMakeCache.txt 里 CMAKE_HOME_DIRECTORY 是
#    D:/oldxiaozhi/... ），Linux 下直接构建会因源码目录不匹配报错。改名保留，不要删：
cd /home/developer/src/xiaozhi/xiaozhi-yuntai
[ -f build/CMakeCache.txt ] && grep -q 'D:/' build/CMakeCache.txt && mv build build.win.bak

# 3) 构建（不要跑 set-target，见 Global Constraints #7）
idf.py build
```

预期输出特征：

- 结尾出现 `Project build complete. To flash, run: idf.py flash`；
- 中间出现 `Generating binary image ...` 与 `xiaozhi.bin binary size 0x...` 一行（工程名 `xiaozhi`，`PROJECT_VER "1.9.0"`）；
- 编译 `main/boards/esp32-s3n16r8-emoji/emoji_board.cc` 时**不得**出现任何提及 `gimbal_mcp_tools.h` 的 warning/error（该头文件只被 `emoji_board.cc` 这一个 TU 包含，它的所有 warning 都会挂在这个 TU 上）；
- 若出现 `esp32s3` 之外的目标或 `Board type ... not found`，说明误跑了 `set-target`，立刻停手并恢复 `sdkconfig`。

### 路径 B：ESP-IDF 环境不可用（当前这台 Linux 机就是这种情况）

事实：本机 `IDF_PATH` 为空、`command -v idf.py` 无输出、`~/esp` 与 `/opt/esp-idf` 都不存在；现有 `build/` 来自 Windows（`D:/esp/Espressif/frameworks/esp-idf-v5.5.3`）。所以默认走本路径：**逐行代码自审清单 + 真机烧录验收（Task 4）**。

自审清单（改完每个任务，逐条打勾，任何一条对不上就回去改代码，不要"看起来对"就过）：

**B1 — 签名匹配**（逐个到源码里核对，行号已核实）
- [ ] `McpServer::AddTool(const std::string& name, const std::string& description, const PropertyList& properties, std::function<ReturnValue(const PropertyList&)> callback)` —— `main/mcp_server.h:261`。四个实参顺序/类型完全一致。
- [ ] `using ReturnValue = std::variant<bool, int, std::string>;` —— `main/mcp_server.h:16`。回调返回类型写 `-> ReturnValue`。
- [ ] `Property(const std::string& name, PropertyType type)` —— `main/mcp_server.h:35`（必填、无范围）。用于 `direction`。
- [ ] `Property(const std::string& name, PropertyType type, int min_value, int max_value)` —— `main/mcp_server.h:45`（必填 + 范围；非 integer 会抛）。用于 `pan` / `tilt`。**注意不要误用 3 参模板构造（那是"带默认值"，会让参数变成可选）。**
- [ ] `void ServoController::HeadMove(int x_offset, int y_offset, int servo_delay = SERVO_DELAY)` —— `servo_controller.h:58`（相对偏移，内部 clamp，逐步 `vTaskDelay`）。
- [ ] `int ServoController::GetCurrentXAngle() const` / `GetCurrentYAngle() const` —— `servo_controller.h:112,118`（inline getter，零开销、不阻塞）。
- [ ] `xTaskCreate(TaskFunction_t, const char*, uint32_t stack_depth, void*, UBaseType_t prio, TaskHandle_t*)`、`xQueueCreate(UBaseType_t len, UBaseType_t item_size)`、`xQueueSend(QueueHandle_t, const void*, TickType_t)`、`xQueueReceive(QueueHandle_t, void*, TickType_t)` 用法与 `emoji_controller.cc:35-48`、`electron_bot_controller.cc:109-117` 一致。

**B2 — 限位值**
- [ ] `board_config.h` 展开值确认：`SERVO_MIN_X = (90-40) = 50`、`SERVO_MAX_X = (90+40) = 130`、`SERVO_MIN_Y = (90-20) = 70`、`SERVO_MAX_Y = (90+20) = 110`。
- [ ] 新代码里 `grep -n "50\|130\|70\|110" gimbal_mcp_tools.h` 只应命中注释/无关数字（如栈大小 4096），**不应命中任何限位字面量**。
- [ ] 方向语义没搞反：`up` 是 **tilt 减小**（`HeadUp(offset)` = `HeadMove(0, -offset)`，`servo_controller.cc:165-167`），`left` 是 **pan 减小**（`HeadLeft` = `HeadMove(-offset, 0)`，`servo_controller.cc:173-175`）。

**B3 — 栈与阻塞**
- [ ] `tools/call` 在独立 pthread 执行，`cfg.stack_size` 默认 `DEFAULT_TOOLCALL_STACK_SIZE = 6144`、`cfg.prio = 1`（`mcp_server.cc:19,351-366`）。因此三个工具回调体内**不得出现** `vTaskDelay`、`HeadMove`、`HeadCenter`、`HeadNod`、`HeadShake`、`HeadRoll`、`PlayAnimation` 任何一个；只允许 `GetCurrentXAngle/GetCurrentYAngle`、算术、`xQueueSend`、`std::to_string`、字符串拼接。
- [ ] worker 任务栈 4096、优先级 4；worker 里才允许调 `HeadMove`（最坏 `set` 从 pan 50 到 130 = 80 步 × 10ms ≈ 800ms）。
- [ ] worker 是常驻死循环 + `xQueueReceive(..., portMAX_DELAY)`，不忙等、不 `vTaskDelete(NULL)`。

**B4 — 去重与注册一次**
- [ ] `McpServer::AddTool` 按名字去重（`mcp_server.cc:110-119`，重名只打 `Tool %s already added` warning 并丢弃）。三个新名字与本板实际注册的通用工具（只有 `self.get_device_status`、`self.audio_speaker.set_volume`；`set_brightness`/`set_theme`/`take_photo`/`set_press_to_talk` 在本板都不注册）无冲突。
- [ ] `GimbalMcpTools::Initialize()` 只在 `EmojiBoard::InitializeIot()` 里被调用一次；`InitializeIot()` 只在 `EmojiBoard` 构造函数里被调用一次（`emoji_board.cc:332`）。全局/静态变量里没有第二个 `GimbalMcpTools` 实例。

**B5 — 返回类型**
- [ ] 每个回调的每条 `return` 都是 `std::string` 表达式（`std::string("...")` 或以 `std::string` 开头的 `+` 链），**绝不写 `return "字面量";`**：`ReturnValue` 是 `std::variant<bool,int,std::string>`，裸 `const char*` 走到 variant 的转换构造是经典陷阱（依赖 P0608 才不会选中 `bool`），不要赌编译器。
- [ ] 三个工具都返回 JSON 形状字符串（`{"pan":N,"tilt":N}` 或 `{"error":"..."}`）；框架会统一包成 `{"content":[{"type":"text","text":"..."}],"isError":false}`（`main/mcp_server.h:226-249`），**没有 `isError:true` 这条路**，所以错误必须写在 text 里让模型看见。

**B6 — 构建系统**
- [ ] 只新增了 `.h`，没新增 `.cc`（见 Global Constraints #5）。若确实新增了 `.cc`，必须记一笔"下次构建前先 `idf.py reconfigure`"。
- [ ] `static constexpr` 类成员在 C++17 隐式 inline，无需类外定义（本仓库 C++ 标准：`emoji_controller.h:311-314` 已在用同一写法）。

---

### Task 1: 云台动作队列与 worker 任务（新建 `gimbal_mcp_tools.h` + 板级接线）

**Files:**
- Create: `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h`
- Modify: `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/emoji_board.cc:13`（新增 include）
- Modify: `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/emoji_board.cc:96-101`（新增成员）
- Modify: `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/emoji_board.cc:274-277`（`InitializeIot()` 落点）
- Modify: `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/emoji_board.cc:344-350`（析构）

**Interfaces:**
- Consumes: `ServoController::HeadMove(int x_offset, int y_offset, int servo_delay = SERVO_DELAY)`；`ServoController::GetCurrentXAngle() const` / `GetCurrentYAngle() const`；`board_config.h` 的 `SERVO_MIN_X/MAX_X/MIN_Y/MAX_Y/CENTER_X/CENTER_Y/SERVO_DELAY`；FreeRTOS `xQueueCreate/xQueueSend/xQueueReceive/xTaskCreate/vQueueDelete/vTaskDelete`。
- Produces: `class GimbalMcpTools`，公开接口只有 `explicit GimbalMcpTools(ServoController*)`、`void Initialize()`、析构。私有：`struct GimbalTarget{int pan;int tilt;}`、`struct QueueResult{bool ok;int pan;int tilt;}`、`static int ClampPan(int)`、`static int ClampTilt(int)`、`static void WorkerTask(void*)`、`QueueResult QueueTarget(int,int)`、`std::string QueueAndDescribe(int,int)`。

**背景（为什么是自建队列，而不是投给 `EmojiController` 的动画队列）**

`EmojiController` 的动画队列确实能跑云台长动作（`AnimationType::HEAD_NOD/HEAD_SHAKE/HEAD_ROLL`，`emoji_controller.cc:183-192`），但它有两个硬伤，导致它**不能**用来承载 MCP 工具：

1. `ExecuteHeadNodAnimation/ExecuteHeadShakeAnimation/ExecuteHeadRollAnimation` 开头都有 `if (emoji_screen_ == nullptr || left_eye_ == nullptr || right_eye_ == nullptr) return;`（`emoji_controller.cc:3025-3028`、`3120-3127` 等）。而 `emoji_screen_` 只在用户**长按 BOOT 进入"表情模式"**后才创建（`emoji_board.cc:216-237` → `CreateEmojiScreen()`）。默认的"对话模式"下，投进去的云台动画会**静默什么都不做**——对模型可见的工具来说这是不可接受的假成功。
2. `AnimationTask` 有 `is_animating_` 门（`emoji_controller.cc:143-147`）：已有动画在跑时，新消息直接**丢弃**。表情联动随时在占用这个门。

所以本任务自建一条只服务云台工具的队列 + 一个 worker 任务（照抄 `electron_bot_controller.cc:104-118` 的 queue+task 范式），它在两种显示模式下都有效，且与表情动画互不设门。队列里只放**绝对目标角度**，`look` 的相对语义在工具回调里换算成绝对值——这样 `look` 与 `set` 共用一条执行路径，clamp 语义只有一份。

**已知限制（写进代码注释，不要试图在本计划里修）**：`EmotionResponseController` 与 `EmojiController` 的动画任务也会直接驱动同一个 `ServoController`（`emotion_response_controller.cc:334-503`、`emoji_controller.cc:3021+`），`ServoController` 没有任何互斥。工具动作与表情联动并发时，两者会互相覆盖目标位置（表现为抖一下或最终位置以最后一次动作为准）。这是 fork 既有设计，本计划不引入锁（改 `ServoController` 会波及表情链路，超出范围）。

- [ ] **Step 1: 明确验证标准**

改完后应能观察到：
1. 编译通过，且 `emoji_board.cc` 这个 TU 没有新增 warning。
2. 上电启动日志里，在 `EmojiBoard` 初始化阶段出现两行（顺序固定，因为 worker 任务优先级 4 > 构造上下文，创建后很快就跑起来）：
   - `I (xxx) GimbalMcp: gimbal worker task started`
   - `I (xxx) GimbalMcp: gimbal MCP tools not registered yet (Task 2)` ——本任务故意还不注册工具，用这行确认落点被执行到。
3. 日志里**不出现** `GimbalMcp: failed to create ...`，也不出现 `Stack canary watchpoint triggered (gimbal_worker)`。
4. `tools/list` 里**还没有** `self.gimbal.*`（本任务不注册），设备其余行为（表情、按键、对话）完全不变。

- [ ] **Step 2: 实现**

新建 `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h`，内容如下（本任务先不含 `RegisterMcpTools()`，Task 2 再补）：

```cpp
/**
 * @file gimbal_mcp_tools.h
 * @brief Expose the 2-axis gimbal (pan/tilt servos) as board level MCP tools.
 *
 * Design notes:
 *  - `tools/call` runs in a dedicated pthread whose default stack is only 6144 bytes and whose
 *    priority is 1 (see main/mcp_server.cc: DEFAULT_TOOLCALL_STACK_SIZE / DoToolCall). A single
 *    ServoController::HeadMove() can busy-step for up to ~800ms (80 steps * SERVO_DELAY), so tool
 *    callbacks must never move the servos themselves.
 *  - Therefore every tool callback only clamps the requested position and pushes an ABSOLUTE
 *    target into this queue; a dedicated worker task performs the movement serially.
 *  - The EmojiController animation queue is deliberately NOT used: its head animations early-return
 *    unless the emoji screen exists (only after a BOOT long press), and its is_animating_ gate drops
 *    queued messages, both of which would silently swallow tool calls.
 *  - Known limitation: EmotionResponseController and the EmojiController animation task drive the
 *    same ServoController without any mutex. Tool driven moves and emotion driven moves can
 *    override each other; the last writer wins. This mirrors the pre-existing board design.
 *  - The angle limits live in board_config.h only (SERVO_MIN_X / SERVO_MAX_X / SERVO_MIN_Y /
 *    SERVO_MAX_Y). Never hardcode them here, not even inside the tool descriptions.
 */

#pragma once

#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>
#include <freertos/task.h>

#include <esp_log.h>

#include <algorithm>
#include <string>

#include "board_config.h"
#include "mcp_server.h"
#include "servo_controller.h"

class GimbalMcpTools {
public:
    explicit GimbalMcpTools(ServoController* servo_controller)
        : servo_controller_(servo_controller) {}

    ~GimbalMcpTools() {
        if (worker_task_handle_ != nullptr) {
            vTaskDelete(worker_task_handle_);
            worker_task_handle_ = nullptr;
        }
        if (target_queue_ != nullptr) {
            vQueueDelete(target_queue_);
            target_queue_ = nullptr;
        }
    }

    GimbalMcpTools(const GimbalMcpTools&) = delete;
    GimbalMcpTools& operator=(const GimbalMcpTools&) = delete;

    /**
     * @brief Create the target queue and the worker task, then register the MCP tools.
     *        Must be called once, from the EmojiBoard constructor, after
     *        ServoController::Initialize() has configured the LEDC channels.
     */
    void Initialize() {
        if (servo_controller_ == nullptr) {
            ESP_LOGE(kLogTag, "no ServoController, gimbal MCP tools disabled");
            return;
        }

        target_queue_ = xQueueCreate(kQueueLength, sizeof(GimbalTarget));
        if (target_queue_ == nullptr) {
            ESP_LOGE(kLogTag, "failed to create the gimbal target queue, tools disabled");
            return;
        }

        if (xTaskCreate(WorkerTask, "gimbal_worker", kWorkerStackSize, this, kWorkerPriority,
                        &worker_task_handle_) != pdPASS) {
            ESP_LOGE(kLogTag, "failed to create the gimbal worker task, tools disabled");
            vQueueDelete(target_queue_);
            target_queue_ = nullptr;
            worker_task_handle_ = nullptr;
            return;
        }

        ESP_LOGI(kLogTag, "gimbal MCP tools not registered yet (Task 2)");
    }

private:
    struct GimbalTarget {
        int pan;
        int tilt;
    };

    struct QueueResult {
        bool ok;
        int pan;
        int tilt;
    };

    static constexpr const char* kLogTag = "GimbalMcp";
    static constexpr int kQueueLength = 4;
    static constexpr int kWorkerStackSize = 4096;
    static constexpr int kWorkerPriority = 4;
    static constexpr int kLookStepDegrees = 20;

    static int ClampPan(int pan) {
        return std::max(SERVO_MIN_X, std::min(SERVO_MAX_X, pan));
    }

    static int ClampTilt(int tilt) {
        return std::max(SERVO_MIN_Y, std::min(SERVO_MAX_Y, tilt));
    }

    static void WorkerTask(void* arg) {
        auto* self = static_cast<GimbalMcpTools*>(arg);
        GimbalTarget target = {SERVO_CENTER_X, SERVO_CENTER_Y};

        ESP_LOGI(kLogTag, "gimbal worker task started");

        while (true) {
            if (xQueueReceive(self->target_queue_, &target, portMAX_DELAY) != pdPASS) {
                continue;
            }

            ServoController* servo = self->servo_controller_;
            int pan_offset = target.pan - servo->GetCurrentXAngle();
            int tilt_offset = target.tilt - servo->GetCurrentYAngle();

            ESP_LOGI(kLogTag, "move gimbal: pan %d -> %d, tilt %d -> %d", servo->GetCurrentXAngle(),
                     target.pan, servo->GetCurrentYAngle(), target.tilt);

            if (pan_offset != 0 || tilt_offset != 0) {
                servo->HeadMove(pan_offset, tilt_offset, SERVO_DELAY);
            }
        }
    }

    /**
     * @brief Clamp the requested absolute position and hand it to the worker task.
     *        Never blocks: on a full queue it drops the request and reports ok == false.
     */
    QueueResult QueueTarget(int pan, int tilt) {
        GimbalTarget target = {ClampPan(pan), ClampTilt(tilt)};

        if (target_queue_ == nullptr) {
            ESP_LOGE(kLogTag, "gimbal target queue is missing");
            return QueueResult{false, target.pan, target.tilt};
        }

        if (xQueueSend(target_queue_, &target, 0) != pdPASS) {
            ESP_LOGW(kLogTag, "gimbal queue full, dropped target pan=%d tilt=%d", target.pan,
                     target.tilt);
            return QueueResult{false, target.pan, target.tilt};
        }

        return QueueResult{true, target.pan, target.tilt};
    }

    /**
     * @brief Queue an absolute target and render the MCP text result for it.
     */
    std::string QueueAndDescribe(int pan, int tilt) {
        QueueResult result = QueueTarget(pan, tilt);
        if (!result.ok) {
            return std::string("{\"error\":\"gimbal busy, too many pending moves, retry later\"}");
        }
        return std::string("{\"pan\":") + std::to_string(result.pan) + ",\"tilt\":" +
               std::to_string(result.tilt) + "}";
    }

    ServoController* servo_controller_ = nullptr;
    QueueHandle_t target_queue_ = nullptr;
    TaskHandle_t worker_task_handle_ = nullptr;
};
```

改动 1 —— `emoji_board.cc:13` 之后新增 include（保持现有 include 顺序，加在板级 include 组末尾）：

```cpp
#include "emoji_controller.h"
#include "servo_controller.h"
#include "emotion_response_controller.h"
#include "gimbal_mcp_tools.h"
```

改动 2 —— `emoji_board.cc:96-101` 的成员块，在 `servo_controller_` 之后加一个成员：

```cpp
    // 表情和舵机控制器
    EmojiController* emoji_controller_ = nullptr;
    ServoController* servo_controller_ = nullptr;
    
    // 云台 MCP 工具（self.gimbal.*）
    GimbalMcpTools* gimbal_mcp_tools_ = nullptr;
    
    // 情感响应控制器
    EmotionResponseController* emotion_controller_ = nullptr;
```

改动 3 —— `emoji_board.cc:274-277` 的 `InitializeIot()` 整体替换（这是板级工具注册的自然落点：它已经在构造函数里被调用，且现在只打一行日志）：

```cpp
    void InitializeIot() {
        // 新的MCP架构不再需要手动初始化Thing，由框架自动管理
        ESP_LOGI(TAG, "新版MCP架构已自动管理设备功能");
        
        // 注册板级云台 MCP 工具（self.gimbal.look / set / get_position）。
        // 注册时机：EmojiBoard 构造期。McpServer::AddCommonTools() 会把通用工具插到列表头部
        // 并把已注册的板级工具接在尾部（mcp_server.cc:31-37,106-107），所以先注册不会被埋掉。
        gimbal_mcp_tools_ = new GimbalMcpTools(servo_controller_);
        gimbal_mcp_tools_->Initialize();
    }
```

改动 4 —— `emoji_board.cc:344-350` 的析构函数，先删云台工具（它会先杀 worker 任务，避免 worker 拿着已释放的 `servo_controller_`）：

```cpp
    ~EmojiBoard() {
        delete gimbal_mcp_tools_;
        delete emoji_controller_;
        delete servo_controller_;
        delete emotion_controller_;
        // 清除全局变量
        g_board_instance = nullptr;
    }
```

- [ ] **Step 3: 编译验证**

Run: `idf.py build`（环境不可用时走「编译验证的两条路径」的路径 B）。

Expected：
- 路径 A：出现 `Project build complete.`；`Generating binary image` + `xiaozhi.bin binary size`；`emoji_board.cc` 无新 warning。
- 路径 B：过 B1（`HeadMove` / `GetCurrentXAngle` / `xTaskCreate` / `xQueueCreate` 签名）、B2（无限位字面量）、B3（`QueueTarget` 里没有任何阻塞调用；worker 栈 4096）、B6（只加了 `.h`）；另加两条本任务专属检查：`InitializeIot()` 在构造函数第 332 行被调用且在 `servo_controller_->Initialize()`（316 行）之后；析构里 `delete gimbal_mcp_tools_;` 在 `delete servo_controller_;` 之前。

---

### Task 2: 注册 `self.gimbal.look` / `self.gimbal.set` / `self.gimbal.get_position`

**Files:**
- Modify: `/home/developer/src/xiaozhi/xiaozhi-yuntai/main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h`（`Initialize()` 尾部改为调用 `RegisterMcpTools()`；新增私有方法 `RegisterMcpTools()`）

**Interfaces:**
- Consumes: `McpServer::GetInstance()`（`main/mcp_server.h:254-257`）；`McpServer::AddTool(const std::string&, const std::string&, const PropertyList&, std::function<ReturnValue(const PropertyList&)>)`（`main/mcp_server.h:261`）；`PropertyList(const std::vector<Property>&)`（`main/mcp_server.h:130`）；`Property(const std::string&, PropertyType)`（`:35`）；`Property(const std::string&, PropertyType, int min, int max)`（`:45`）；`Property::value<T>()`（`:70-73`）；`using ReturnValue = std::variant<bool,int,std::string>`（`:16`）。
- Produces: 三个 MCP 工具（工具名 + 参数是跨仓库契约）：
  - `self.gimbal.look`，必填 `direction`（string，取值 `left|right|up|down|center`），返回 `{"pan":N,"tilt":N}`（目标位置）或 `{"error":"..."}`。
  - `self.gimbal.set`，必填 `pan`（integer，`minimum=SERVO_MIN_X` / `maximum=SERVO_MAX_X`）与 `tilt`（integer，`minimum=SERVO_MIN_Y` / `maximum=SERVO_MAX_Y`），返回 `{"pan":N,"tilt":N}`。
  - `self.gimbal.get_position`，无参数，返回 `{"pan":N,"tilt":N}`（当前实际角度）。
- Produces（私有）：`void RegisterMcpTools()`。

**关键语义（务必与 nomifun 侧文档一致）**
- `pan` 小 = 左、大 = 右；`tilt` 小 = 抬头、大 = 低头；`SERVO_CENTER_X/Y` = 居中。
- `look` 是**相对**动作（每次约 `kLookStepDegrees` = 20°，可重复调用直到限位），`center` 是**绝对**回中；`set` 是**绝对**定位。
- `set` 的越界值**根本到不了回调**：`DoToolCall` 在起线程前用 `Property::set_value<int>` 做范围校验，越界抛异常并回 JSON-RPC `error`（`mcp_server.cc:320-348`、`mcp_server.h:75-87`）。所以 nomifun 侧对越界会收到 `{"error":{"message":"Value exceeds maximum allowed: 130"}}` 这种**协议级错误**，不是 `isError` 结果。回调内的 `ClampPan/ClampTilt` 是给 `look` 的相对累加兜底用的。
- 三个工具都**立即返回**，返回的是"目标/当前"位置而非"已完成"；模型若要确认落点，隔一会儿调 `get_position`。

- [ ] **Step 1: 明确验证标准**

改完后应能观察到：
1. 编译通过，无新 warning。
2. 启动日志里出现（`AddTool` 每次注册都会打 `Add tool: ...`，`mcp_server.cc:117`）：
   - `I (xxx) MCP: Add tool: self.gimbal.look`
   - `I (xxx) MCP: Add tool: self.gimbal.set`
   - `I (xxx) MCP: Add tool: self.gimbal.get_position`
   - `I (xxx) GimbalMcp: gimbal MCP tools registered: pan 50-130, tilt 70-110`（数字由宏拼出，正好交叉验证限位）
   - **不出现** `W ... MCP: Tool self.gimbal.xxx already added`（重复注册）。
3. 服务端 `tools/list` 的结果里三个工具都在，且：
   - `self.gimbal.set` 的 `inputSchema.properties.pan` 含 `"minimum":50,"maximum":130`，`tilt` 含 `"minimum":70,"maximum":110`；
   - `inputSchema.required` 为 `["pan","tilt"]`（因为都没有默认值）；
   - `self.gimbal.look` 的 `required` 为 `["direction"]`；
   - `self.gimbal.get_position` 的 `inputSchema.properties` 为 `{}` 且无 `required`。
4. `tools/list` 单页装得下（8000 字节上限，`mcp_server.cc:258`）：本板通用工具**只有两个** —— `self.get_device_status` + `self.audio_speaker.set_volume`（无背光 → 无 `set_brightness`；无摄像头 → 无 `take_photo`；`OledDisplay` 从不设置 `current_theme_name_`，`GetTheme()` 返回空串 → 连 `self.screen.set_theme` 也不注册，见 `mcp_server.cc:59-104` 与 `main/display/oled_display.cc`）。加三个云台工具合计约 2.5KB，响应里**不应**出现 `nextCursor`。
5. `tools/call` `self.gimbal.look` 参数 `direction:"left"` → 立刻收到 `{"content":[{"type":"text","text":"{\"pan\":70,\"tilt\":90}"}],"isError":false}`（从居中开始），随后舵机才转过去；`direction:"sideways"` → `text` 为 `{"error":"invalid direction, ..."}`。

- [ ] **Step 2: 实现**

改动 1 —— `Initialize()` 的最后一行日志替换为真正的注册调用：

```cpp
        RegisterMcpTools();
        ESP_LOGI(kLogTag, "gimbal MCP tools registered: pan %d-%d, tilt %d-%d", SERVO_MIN_X,
                 SERVO_MAX_X, SERVO_MIN_Y, SERVO_MAX_Y);
    }
```

（即把 Task 1 里那行 `ESP_LOGI(kLogTag, "gimbal MCP tools not registered yet (Task 2)");` 整行替换成上面两条语句。）

改动 2 —— 在 `private:` 区、`QueueAndDescribe()` 之后、成员变量声明之前，插入完整的 `RegisterMcpTools()`：

```cpp
    /**
     * @brief Register the three board level gimbal tools. Tool names and argument names are a
     *        cross-repository contract with nomifun's MCP bridge: do not rename them.
     *        Descriptions are built at runtime from the board_config.h limits so that they can
     *        never drift away from the real clamp values.
     */
    void RegisterMcpTools() {
        auto& mcp_server = McpServer::GetInstance();

        const std::string pan_range =
            std::to_string(SERVO_MIN_X) + ".." + std::to_string(SERVO_MAX_X);
        const std::string tilt_range =
            std::to_string(SERVO_MIN_Y) + ".." + std::to_string(SERVO_MAX_Y);
        const std::string center_position =
            std::to_string(SERVO_CENTER_X) + "/" + std::to_string(SERVO_CENTER_Y);

        mcp_server.AddTool(
            "self.gimbal.look",
            "Turn the robot's head (a 2-axis pan/tilt gimbal) to look somewhere. `direction` must be "
            "one of `left`, `right`, `up`, `down`, `center`. `left`, `right`, `up` and `down` turn "
            "about " + std::to_string(kLookStepDegrees) +
                " degrees relative to the current position, so call this tool again to keep turning "
                "until the mechanical limit is reached. `center` moves the head back to the center "
                "position (pan/tilt " + center_position +
                "). The movement runs in the background and takes up to 1 second; this tool returns "
                "immediately with the target position as JSON, for example {\"pan\":70,\"tilt\":90}. "
                "Use `self.gimbal.get_position` afterwards if you need the settled position.",
            PropertyList({Property("direction", kPropertyTypeString)}),
            [this](const PropertyList& properties) -> ReturnValue {
                const std::string direction = properties["direction"].value<std::string>();
                int pan = servo_controller_->GetCurrentXAngle();
                int tilt = servo_controller_->GetCurrentYAngle();

                if (direction == "left") {
                    pan -= kLookStepDegrees;
                } else if (direction == "right") {
                    pan += kLookStepDegrees;
                } else if (direction == "up") {
                    tilt -= kLookStepDegrees;
                } else if (direction == "down") {
                    tilt += kLookStepDegrees;
                } else if (direction == "center") {
                    pan = SERVO_CENTER_X;
                    tilt = SERVO_CENTER_Y;
                } else {
                    ESP_LOGW(kLogTag, "self.gimbal.look: invalid direction: %s", direction.c_str());
                    return std::string(
                        "{\"error\":\"invalid direction, expected one of: left, right, up, down, "
                        "center\"}");
                }

                return QueueAndDescribe(pan, tilt);
            });

        mcp_server.AddTool(
            "self.gimbal.set",
            "Move the robot's head (a 2-axis pan/tilt gimbal) to an absolute position. `pan` is the "
            "horizontal angle in degrees, range " + pan_range + ": " + std::to_string(SERVO_MIN_X) +
                " is fully left, " + std::to_string(SERVO_CENTER_X) + " is centered, " +
                std::to_string(SERVO_MAX_X) + " is fully right. `tilt` is the vertical angle in "
                "degrees, range " + tilt_range + ": " + std::to_string(SERVO_MIN_Y) +
                " looks up, " + std::to_string(SERVO_CENTER_Y) + " is centered, " +
                std::to_string(SERVO_MAX_Y) + " looks down. Values outside those ranges are "
                "rejected. The movement runs in the background and takes up to 1 second; this tool "
                "returns immediately with the target position as JSON, for example "
                "{\"pan\":" + std::to_string(SERVO_CENTER_X) + ",\"tilt\":" +
                std::to_string(SERVO_CENTER_Y) + "}.",
            PropertyList({Property("pan", kPropertyTypeInteger, SERVO_MIN_X, SERVO_MAX_X),
                          Property("tilt", kPropertyTypeInteger, SERVO_MIN_Y, SERVO_MAX_Y)}),
            [this](const PropertyList& properties) -> ReturnValue {
                return QueueAndDescribe(properties["pan"].value<int>(),
                                        properties["tilt"].value<int>());
            });

        mcp_server.AddTool(
            "self.gimbal.get_position",
            "Get the current position of the robot's head (a 2-axis pan/tilt gimbal) as JSON, for "
            "example {\"pan\":" + std::to_string(SERVO_CENTER_X) + ",\"tilt\":" +
                std::to_string(SERVO_CENTER_Y) + "}. `pan` is the horizontal angle (" + pan_range +
                ", lower is more to the left), `tilt` is the vertical angle (" + tilt_range +
                ", lower looks further up), and " + center_position + " is the center position.",
            PropertyList(),
            [this](const PropertyList& properties) -> ReturnValue {
                return std::string("{\"pan\":") +
                       std::to_string(servo_controller_->GetCurrentXAngle()) + ",\"tilt\":" +
                       std::to_string(servo_controller_->GetCurrentYAngle()) + "}";
            });
    }
```

- [ ] **Step 3: 编译验证**

Run: `idf.py build`（环境不可用时走路径 B）。

Expected：
- 路径 A：`Project build complete.`；无新 warning（特别是**没有** `-Wparentheses` 或 variant 相关的 note；若出现 `no viable conversion from 'const char*' to 'ReturnValue'` 之类，说明某个 `return` 忘了包 `std::string`）。
- 路径 B：过 B1（`AddTool` 四参签名、`Property` 两种构造、`value<int>()` / `value<std::string>()`）、B2（描述文本全部由 `std::to_string(宏)` 拼出，`grep` 不到限位字面量；`up` 减 tilt、`left` 减 pan）、B3（三个回调体内只有 getter/算术/`xQueueSend`/字符串拼接，无 `vTaskDelay`/`Head*`）、B4（三个名字唯一，`RegisterMcpTools()` 只被 `Initialize()` 调一次）、B5（每条 `return` 都是 `std::string` 表达式）。
- 追加一条本任务专属检查：`Property("pan", kPropertyTypeInteger, SERVO_MIN_X, SERVO_MAX_X)` 用的是 **4 参** `(name,type,min,max)` 构造（必填 + 范围），**不是** 3 参模板构造（那会让参数变成"有默认值 → 可选"，`GetRequired()` 就不会把它放进 `required`，模型可以省略参数）。

---

### Task 3: 编译收口与静态自审（含无 ESP-IDF 环境时的替代路径）

**Files:**
- Modify: 无（本任务不改代码；若自审发现问题，回到 Task 1/2 对应位置修）

**Interfaces:**
- Consumes: Task 1/2 的产物（`gimbal_mcp_tools.h`、`emoji_board.cc` 的 4 处改动）。
- Produces: 一份可判定的"可烧录"结论 —— 要么 `idf.py build` 成功，要么 B1-B6 全绿 + 本任务的 grep 断言全过。

- [ ] **Step 1: 明确验证标准**

- 路径 A 可用：`idf.py build` 成功，`build/xiaozhi.bin` 存在且 mtime 是刚才；`grep -c "gimbal" build/compile_commands.json` 不为 0（说明 `emoji_board.cc` 确实被重编）。
- 路径 A 不可用：B1-B6 全部打勾，且下面 Step 2 的 5 条 grep 断言全部符合预期输出。
- 两条路径都要满足的"零回归"标准：`main/` 目录下除 `boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h`（新增）与 `boards/esp32-s3n16r8-emoji/emoji_board.cc`（4 处）以外，**没有任何文件被修改**；没有新增任何 `.cc`。

- [ ] **Step 2: 实现（执行验证命令）**

```bash
cd /home/developer/src/xiaozhi/xiaozhi-yuntai

# 环境探测：决定走路径 A 还是 B
echo "IDF_PATH=${IDF_PATH:-<empty>}"; command -v idf.py || echo "no idf.py on PATH"
ls "$HOME/esp/esp-idf/export.sh" /opt/esp-idf/export.sh 2>/dev/null || echo "no ESP-IDF install found"

# ---------- 路径 A（仅在上面探测到 ESP-IDF 时执行） ----------
# . "$HOME/esp/esp-idf/export.sh"
# [ -f build/CMakeCache.txt ] && grep -q 'D:/' build/CMakeCache.txt && mv build build.win.bak
# idf.py build
# ls -l build/xiaozhi.bin

# ---------- 路径 B：静态断言（无论走哪条路径都跑，很快） ----------
# B-1 只改了预期的两个文件、没新增 .cc（对比 sdkconfig 的 mtime 作参照，仓库非 git 无法 diff）
ls -l --time-style=+%F_%T main/boards/esp32-s3n16r8-emoji/*.h main/boards/esp32-s3n16r8-emoji/*.cc
ls main/boards/esp32-s3n16r8-emoji/*.cc | wc -l   # 期望 4：emoji_board / emoji_controller / emotion_response_controller / servo_controller

# B-2 限位字面量不得出现在新代码里（期望：无输出）
grep -nE '\b(50|130|70|110)\b' main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h

# B-3 工具回调里不得有阻塞调用（期望：只命中注释里的说明文字，不命中语句）
grep -nE 'vTaskDelay|HeadMove|HeadCenter|HeadNod|HeadShake|HeadRoll|PlayAnimation' \
  main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h

# B-4 三个工具名逐字正确、各出现一次（期望各 1 行）
grep -n 'self\.gimbal\.' main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h

# B-5 没有裸字面量 return（期望：无输出）
grep -nE 'return[[:space:]]+"' main/boards/esp32-s3n16r8-emoji/gimbal_mcp_tools.h

# B-6 通用层没被碰过：这几个文件里不应出现 gimbal 字样（期望：无输出）
grep -rn 'gimbal' main/mcp_server.h main/mcp_server.cc main/application.cc main/CMakeLists.txt main/protocols/
```

预期输出逐条对照：

| 断言 | 期望 |
|---|---|
| B-1 | 板目录下恰好 4 个 `.cc`（未新增）；`gimbal_mcp_tools.h` 存在 |
| B-2 | 无输出（限位只以宏出现） |
| B-3 | 只命中文件头注释里解释设计的那几行 + `WorkerTask` 里那一处 `servo->HeadMove(...)`；**不得**命中任何 lambda 内部 |
| B-4 | 恰好 3 行：`self.gimbal.look` / `self.gimbal.set` / `self.gimbal.get_position` |
| B-5 | 无输出 |
| B-6 | 无输出 |

- [ ] **Step 3: 编译验证**

Run: `idf.py build`（路径 A）；不可用时以上面 6 条断言 + B1-B6 自审清单全绿作为等价结论，并在任务收尾时明确写下"未编译验证，理由：本机无 ESP-IDF；改由 Task 4 真机烧录验收兜底"。

Expected：
- 路径 A：`Project build complete.`，`build/xiaozhi.bin` 新生成。
- 路径 B：6 条断言全部符合上表；任何一条不符 → 回 Task 1/2 修代码，不要放行到 Task 4。

---

### Task 4: 真机烧录与验收（spec §12 真机验收）

**Files:**
- Modify: 无（纯验收；若验收失败，按现象回到 Task 1/2 改代码）

**Interfaces:**
- Consumes: Task 3 产出的固件；nomifun 侧的 `/robot/ota`、`/robot/v1`、伙伴"远程控制"Tab 的机器人连接节（Plan A/B 的产物）。
- Produces: 一份逐项打勾的验收结论 + 观测到的 `pan`/`tilt` 实测限位（供 nomifun 侧文案与提示词参考）。

- [ ] **Step 1: 明确验证标准**

七条，全部满足才算通过：
1. 烧录后串口出现三条 `MCP: Add tool: self.gimbal.*` 与 `GimbalMcp: gimbal MCP tools registered: pan 50-130, tilt 70-110`、`GimbalMcp: gimbal worker task started`。
2. 配网指向 nomifun 后设备能连上并屏显 6 位激活码；在伙伴远程 Tab 输码绑定成功。
3. nomifun 侧 `tools/list` 拿到三个云台工具（日志或 UI 可见），响应无 `nextCursor`。
4. 对伙伴说"向左看" → 舵机水平向左转约 20°，串口出现 `GimbalMcp: move gimbal: pan 90 -> 70, tilt 90 -> 90`；连续说三次 → 第三次停在 `pan 50` 不再动（撞限位），且**无啸叫、无堵转发热**。
5. 说"把头转到最右边、稍微低头" → 模型应调 `self.gimbal.set`（pan 130 / tilt 105 之类），舵机平滑到位；说一个越界值（如"pan 转到 200"）→ nomifun 侧应看到协议级错误 `Value exceeds maximum allowed: 130`，**舵机不动**、设备不重启。
6. 长动作不阻塞对话：从 `pan 50` 一把 `set` 到 `pan 130`（约 800ms）期间，伙伴的语音回复继续流畅播放，`tools/call` 的响应在日志时间戳上是**毫秒级**返回（不是 800ms 后才回），串口无 `task_wdt`、无 `Stack canary watchpoint triggered (tool_call)`。
7. 与表情联动共存：说一句让伙伴表现情绪的话（触发 `llm.emotion` → 表情动画也会动舵机），随后立刻调 `self.gimbal.look center` → 头最终回到居中，设备不崩、不复位（并发下中途抖动属已知限制，见 Task 1 背景）。

- [ ] **Step 2: 实现（烧录与验收操作）**

```bash
# ---- 烧录（Linux 侧，ESP-IDF 可用时）----
cd /home/developer/src/xiaozhi/xiaozhi-yuntai
. "$HOME/esp/esp-idf/export.sh"
ls /dev/ttyUSB* /dev/ttyACM* 2>/dev/null          # 确认串口号
idf.py -p /dev/ttyACM0 flash monitor              # 退出 monitor: Ctrl+]

# ---- 烧录（Windows 侧，即上次构建 build/ 的那台机）----
# idf.py -p COM<N> flash monitor

# ---- 抓关键日志（monitor 里肉眼看即可；要留档可另开一个终端）----
# idf.py -p /dev/ttyACM0 monitor | tee /tmp/gimbal-acceptance.log
```

验收流程（按 spec §12「真机验收」）：

1. **配网指向 nomifun**：设备首次上电（或长按复位进配网）→ 连它的配网热点 → 进配网页「高级设置」→ OTA 地址填 `http://<桌面机 LAN IP>:<端口>/robot/ota`（端口以 nomifun「添加机器人」弹窗显示为准；LAN 监听器默认 25808，且必须先打开"局域网访问"开关）→ 保存重启。
2. **绑定伙伴**：设备屏显并朗读 6 位激活码 → 桌面端进入目标伙伴 → 远程控制 Tab → 机器人连接 → 添加机器人 → 输入 6 位码 → 列表出现该设备且状态药丸为在线。
3. **让伙伴调用云台工具**：对着机器人依次说（每句之间等回复播完）：
   - "向左看" / "向右看" / "抬头看看" / "低头" / "头转回中间"
   - "把头转到最右边，然后稍微低一点头"
   - "你现在头朝哪边？"（应触发 `self.gimbal.get_position`，模型口述当前 pan/tilt）
4. **观察舵机动作与限位**：每次调用后核对串口 `GimbalMcp: move gimbal: pan A -> B, tilt C -> D` 的 B/D 是否落在 `[50,130]` / `[70,110]` 内；连续同方向 3 次后应停在限位值上并保持静止（没有嗡嗡声）。
5. **确认长动作不阻塞对话**：让伙伴"先说一段长一点的话，同时把头从最左转到最右"——观察语音是否连续无卡顿、`tools/call` 回复时间戳与请求时间戳的差是否 < 50ms。
6. **异常路径**：诱导一次越界（对伙伴说"把 pan 设成 200"）与一次非法方向（若模型愿意传 `direction:"backwards"`；否则跳过并在结论里注明未覆盖）→ 记录 nomifun 侧看到的错误文案。

- [ ] **Step 3: 编译验证**

Run: 本任务不再编译；以 Step 1 的七条验收标准逐条打勾作为收尾。若验收失败，按现象定位：

| 现象 | 定位 |
|---|---|
| 无 `Add tool: self.gimbal.*` 日志 | `InitializeIot()` 未调 `Initialize()`，或 `xQueueCreate`/`xTaskCreate` 失败（看有无 `failed to create`）→ 回 Task 1/2 |
| `tools/list` 里没有云台工具但有 `Add tool` 日志 | 8000 字节分页把它们挤到第二页 → 检查 nomifun 桥是否处理 `nextCursor`（nomifun 侧问题，不是固件） |
| 工具返回值对但舵机不动 | worker 任务没起来（无 `gimbal worker task started`），或 `servo_controller_` 为空 → 回 Task 1 |
| 调用时对话卡住约 1 秒 | 某个回调里残留了同步 `HeadMove`（B3 漏检）→ 回 Task 2 |
| `Stack canary watchpoint triggered (gimbal_worker)` | 把 `kWorkerStackSize` 提到 6144 并重测（`HeadMove` + `ESP_LOGI` 的 printf 是主要消耗） |
| 舵机堵转啸叫 | 目标越过了机械可动范围 → 收紧 `board_config.h` 的 `SERVO_MIN_*/MAX_*`（限位唯一真相在那里，改完描述与 schema 自动跟随），**不要**在 `gimbal_mcp_tools.h` 里补 clamp |

---

## 跨仓库契约清单（交给 nomifun 侧核对，勿改）

| 工具名 | 参数 | 类型 | 取值 | 返回（text 内容） |
|---|---|---|---|---|
| `self.gimbal.look` | `direction` | string，必填 | `left` \| `right` \| `up` \| `down` \| `center`（相对转约 20°；`center` 为绝对回中） | `{"pan":N,"tilt":N}`（目标位置）或 `{"error":"invalid direction, expected one of: left, right, up, down, center"}` |
| `self.gimbal.set` | `pan` | integer，必填，`minimum:50` `maximum:130` | 50=最左，90=居中，130=最右 | `{"pan":N,"tilt":N}`（目标位置） |
| | `tilt` | integer，必填，`minimum:70` `maximum:110` | 70=最上（抬头），90=居中，110=最下（低头） | 同上 |
| `self.gimbal.get_position` | — | — | — | `{"pan":N,"tilt":N}`（当前实际角度） |

补充给 nomifun 侧的行为约定：
- 三个工具都**立即返回**（毫秒级），返回的是目标/当前位置，不代表动作已完成；一次 `set` 最长约 800ms 才停稳。
- `pan`/`tilt` 越界由固件通用层在起线程前拦截，nomifun 会收到 JSON-RPC `error.message`（形如 `Value exceeds maximum allowed: 130`），**不是** `isError` 结果；`direction` 非法则是正常结果里的 `{"error":...}`。
- 所有结果都被固件包成 `{"content":[{"type":"text","text":"<上表内容>"}],"isError":false}`。
