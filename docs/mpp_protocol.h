/**
 * @file    mpp_protocol.h
 * @brief   WPC Qi MPP (Magnetic Power Profile) Protocol Data Packet Structures
 * @version 2.7 (Ultimate Edition: Strict Alignment, Full Headers, Deep Comments)
 * @note    所有多字节字段 (uint16_t, int16_t, uint32_t) 在空中均以大端模式 (BIG-ENDIAN) 传输。
 *          小端架构的 MCU 在解析时必须使用 ntohs() / ntohl() 进行字节序转换。
 */

#ifndef MPP_PROTOCOL_H
#define MPP_PROTOCOL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define GET_PAYLOAD_LEN(header) \
    ((header) <= 0x1F ? (1 + ((header) / 32)) : \
    ((header) <= 0x7F ? (2 + ((header) - 32) / 16) : \
    ((header) <= 0xDF ? (8 + ((header) - 128) / 8) : \
                        (20 + ((header) - 224) / 4))))

#pragma pack(push, 1)

/* ========================================================================= *
 *             DIRECTION 1: PRX -> PTX (ASK MODULATION MESSAGES)             *
 * ========================================================================= */
/* 注意：0x01-0x09 与标准 BPP 完全一致，此处不再冗余重复定义，重点提供 MPP 扩展结构 */

/* Header: 0x13 - Mode Select Request (MSR) 模式选择请求 [Size: 1 Byte] */
typedef struct {
    /** [b7-b6]: 偏好 (0=无, 1=保留合约, 2=不保留)
     *  [b5]: Rsvd, [b4-b3]: 主模式 (0=CPM, 1=NPM, 2=LPM, 3=HPM)
     *  [b2-b1]: Rsvd, [b0]: 辅模式 (0=未选择, 1=选择对应的辅模式) */
    uint8_t mode_cfg;
} mpp_rx_msr_t;

/* Header: 0x18 - Cloak Request (CLOAK) 休眠请求 [Size: 1 Byte] */
typedef struct {
    /** 必须为 0x00。触发进入空窗期(Cloak Phase)。 */
    uint8_t reserved;
} mpp_rx_cloak_t;

/* Header: 0x19 - Extended Control Error (XCE) 扩展控制误差 [Size: 1 Byte] */
typedef struct {
    /** 扩展的控制误差值，具有更高的分辨率。8位有符号整数 (补码)。 */
    int8_t extended_ce;
} mpp_rx_xce_t;

/* Header: 0x20 - Specific Request (SRQ) 特定请求扩展 [Size: 2 Bytes] */
typedef struct {
    /** 请求代码: 0xA0(PLA格式), 0xA1(XCE计算法), 0xA7(版本), 0xA9(增益),
     *  0xF0(频率), 0xF3(功率级), 0xF5/0xF7(Cloak延时), 0xF6(控制特性) 等。 */
    uint8_t request_code;
    /** 参数。 */
    uint8_t parameter;
} mpp_rx_srq_t;

/* Header: 0x23 - Calibration Operation (CAL_OP) 校准操作 [Size: 2 Bytes] */
typedef struct {
    /** 操作代码: 例如 0x01=COMMIT(提交校准结果)。 */
    uint8_t operation;
    /** 附加参数。 */
    uint8_t parameter;
} mpp_rx_cal_op_t;

/* Header: 0x28 - Get Request (GET) 获取请求 [Size: 2 Bytes] */
typedef struct {
    /** 必须为 0x00。 */
    uint8_t rsvd;
    /** 请求PTx数据包代号: 0=XID, 2=INV, 3=PLAP, 12=MATEDQ_RES, 15=GMP。 */
    uint8_t parameter;
} mpp_rx_get_t;

/* Header: 0x29 - Enabled Data Streams (EDS) 启用数据流掩码 [Size: 2 Bytes] */
typedef struct {
    /** 16位数据流掩码 (大端)。Bit 1=支持鉴权流, Bit 2=支持主动散热流。 */
    uint16_t streams_bitmask;
} mpp_rx_eds_t;

/* Header: 0x2C - Enter Calibration (CAL_ENTER) 进入校准 [Size: 2 Bytes] */
typedef struct {
    /** 协议预留参数。严格分配长度 2 字节数组以对齐 Payload。 */
    uint8_t reserved_payload[4];
} mpp_rx_cal_enter_t;

/* Header: 0x2D - Exit Calibration (CAL_EXIT) 退出校准 [Size: 2 Bytes] */
typedef struct {
    /** [b7-b1]: Rsvd, [b0]: Clear (0=保留校准点, 1=清除校准点)。 */
    uint8_t clear_flag;
    /** 必须为 0x00，满 2 Bytes。 */
    uint8_t reserved;
} mpp_rx_cal_exit_t;

/* Header: 0x38 - Simultaneous Data Stream Response (SDSR PRx) 并发流响应 [Size: 3 Bytes] */
typedef struct {
    /** 必须为 0x00。 */
    uint8_t selector_rsvd;
    /** [b7-b4]: Rsvd, [b3-b0]: Stream Number (流通道号)。 */
    uint8_t stream_number;
    /** [b7-b4]: Rsvd, [b3-b0]: Type (0=ACK, 1=UNEXPECTED, 2=ERR_BUSY, 3=ERR_CRC)。 */
    uint8_t type_cmd;
} mpp_rx_sdsr_t;

/* Header: 0x48 - Simultaneous Auxiliary Data Control (SADC) PRx并发辅助流控制 [Size: 4 Bytes]
 * Figure 121 / Table 63. Response: PTx may continue streams; PRx opens/closes transports.
 */
typedef struct {
    /** [b7-b3]: Rsvd(=0), [b2-b0]: Request (Table 63: 0=reset all … 4=open, 5–7 reserved). */
    uint8_t request;
    /** [b7-b5]: Rsvd(=0), [b4-b0]: Stream Number (Table 30). */
    uint8_t stream_number;
    /** 16-bit Parameter (Table 64), big-endian. */
    uint16_t parameter;
} mpp_rx_sadc_t;

/* Header: 0x50 - K-est Coefficients (KEST_COEFF) K值估算系数 [Size: 5 Bytes] */
typedef struct {
    /** [b7-b1]: Rsvd, [b0]: Selector (0 = 代表 128kHz HPM)。 */
    uint8_t selector;
    /** Alpha_0r 系数。真实值 = 字段值 / 100。有符号8位。 */
    int8_t alpha_0r;
    /** Alpha_1r 系数。真实值 = 字段值 / 100。有符号8位。 */
    int8_t alpha_1r;
    /** 必须为 0x00，补满 5 Bytes。 */
    uint8_t reserved[4];
} mpp_rx_kest_coeff_t;

/* Header: 0x58 (Selector = 0: REPORT, 1: PLA) 复用功率核算包 [Size: 5 Bytes] */
typedef struct {
    /** 0x00=REPORT (报告基本ID), 0x01=PLA (基准功率损耗核算)。 */
    uint8_t selector;
    /** PLA模式为 Received Power (1mW，大端)。 */
    uint16_t b1_b2;
    /** PLA模式为 P_rect (1mW，大端)。 */
    uint16_t b3_b4;
} mpp_rx_pla_report_t;

/* Header: 0x78 - Power Loss Accounting Parameters (PLAP) MPP校准参数 [Size: 7 Bytes] */
typedef struct {
    /** 保留位。 */
    uint8_t rsvd;
    /** Friend Metal 系数。真实值 = 字段值 * 0.5 mΩ。有符号16位大端。 */
    int16_t alpha_fm;
    /** Friend Metal 恒定损耗。真实值 = 字段值 * 0.5 mW。有符号16位大端。 */
    int16_t alpha_fm_dc;
    /** 接收端线圈等效转换系数。真实值 = 字段值 * 0.0001。有符号16位大端。 */
    int16_t g_coil_t;
} mpp_rx_plap_t;

/* Header: 0x81 - Extended Identification (MPP-XID) MPP扩展身份标识 [Size: 8 Bytes] */
typedef struct {
    /** MPP 中 B0 必须绝对固定为 0xFE。 */
    uint8_t fixed_fe;
    /** [b7]: Restricted (1 = 受限模式工作), [b6-b0]: 制造商保留。 */
    uint8_t restricted_mfg;
    /** 制造商保留。 */
    uint8_t mfg_rsvd;
    /** 测量的整流电压 VRECT。单位: 20 mV。(例如 250 代表 5V)。无符号8位。 */
    uint8_t v_rect;
    /** 生态系统缩放系数 alpha_0r。真实值 = 字段值 / 100。有符号8位。 */
    int8_t alpha_0r;
    /** 生态系统缩放系数 alpha_1r。真实值 = 字段值 / 100。有符号8位。 */
    int8_t alpha_1r;
    /** 阈值常数。强制固定为 100。 */
    int8_t alpha_k_thr;
    /** 制造商保留。 */
    uint8_t mfg_rsvd2;
} mpp_rx_xid_t;

/* Header: 0x84 - Extended Capabilities (ECAP) PRx扩展能力 [Size: 8 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t rsvd;
    /** [b7-b4]: Rsvd, [b3-b0]: 最低充电功率要求。 */
    uint8_t min_charge_pwr;
    /** 保留。 */
    uint8_t rsvd2;
    /** [b7-b5]: 并发流数量, [b4-b2]: 数据流缓冲大小, [b1-b0]: Rsvd。 */
    uint8_t data_streams_info;
    /** 4 字节保留位，使用数组填充满 8 Bytes 定长。 */
    uint8_t mfg_rsvd[5];
} mpp_rx_ecap_t;

/* Header: 0x88 - Power Loss Accounting 2 (PLA_2) 精确功损核算2 [Size: 9 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t rsvd;
    /** 测量接收功率。单位: 1 mW，大端。 */
    uint16_t p_received;
    /** 测量整流功率。单位: 1 mW，大端。 */
    uint16_t p_rect;
    /** 测量整流电压。单位: 1 mV，大端。 */
    uint16_t v_rect;
    /** 测量整流电流。单位: 1 mA，大端。 */
    uint16_t i_rect;
} mpp_rx_pla2_t;

/* Header: 0x90 - Power Loss Accounting Parameters 2 (PLAP_2) 精确功损参数2 [Size: 10 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t rsvd;
    /** 二代接收端线圈系数。无符号16位大端。 */
    uint16_t g_coil_t;
    /** 友好金属补偿系数 ITX。有符号16位大端 (补码)。 */
    int16_t alpha_fm_itx;
    /** 友好金属补偿系数 IRECT。有符号16位大端 (补码)。 */
    int16_t alpha_fm_irect;
    /** 保留。 */
    uint8_t rsvd2;
    /** 友好金属补偿系数 VRECT。有符号16位大端 (补码)。 */
    int16_t alpha_fm_vrect;
} mpp_rx_plap2_t;

/* Header: 0x96 - Calibration Capture (CAL_CAPTURE) 请求捕获校准点 [Size: 10 Bytes] */
typedef struct {
    /** 校准点索引 (Calibration Point Index)。例如 1=轻载, 2=中载。 */
    uint8_t cal_point_idx;
    /** 具体的操作要求。 */
    uint8_t operation;
    /** 协议规定后 8 Bytes 全填充 0x00，使用数组满 10 Bytes。 */
    uint8_t reserved[3];
} mpp_rx_cal_capture_t;

/* Header: 0xA8 - Mated-Q Coefficients (MATEDQ_COEFF) 配对Q值防异物系数 [Size: 13 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t rsvd;
    /** 判定方程系数 g0。真实值 = 字段值 * 0.001。有符号16位大端。 */
    int16_t g0;
    /** 判定方程系数 g1。真实值 = 字段值 * 0.001。有符号16位大端。 */
    int16_t g1;
    /** PRx自带误差距离 d0。真实值 = 字段值 * 0.001。有符号16位大端。 */
    int16_t d0;
    /** 6字节保留位占位符，使用数组满 13 Bytes 定长。 */
    uint8_t rsvd_pad[2];
} mpp_rx_matedq_coeff_t;


/* ========================================================================= *
 *             DIRECTION 2: PTX -> PRX (FSK MODULATION MESSAGES)             *
 * ========================================================================= */

/* Header: 0x01 - Error Status (ERR) 错误状态报告 [Size: 1 Byte] */
typedef struct {
    /** [b7-b4]: Info (1=缺HPM系数, 2=测量错误), [b3]: Rsvd, [b2-b0]: Error Code (1=无法计算)。 */
    uint8_t error_info;
} mpp_tx_err_t;

/* Header: 0x0A - End Power Transfer Request (EPTR) 结束功率请求 [Size: 1 Byte] */
typedef struct {
    /** 结束原因代码 (例如 0=模式切换)。 */
    uint8_t reason_code;
} mpp_tx_eptr_t;

/* Header: 0x13 - Mode Selection Notification (MSN) 模式选择通知 [Size: 1 Byte] */
typedef struct {
    /** [b7-b4]: Rsvd, [b3-b2]: Main Mode (0=CPM, 1=NPM, 2=LPM, 3=HPM), [b1-b0]: Rsvd。 */
    uint8_t mode_status;
} mpp_tx_msn_t;

/* Header: 0x14 - Calibration Capture Response (CAL_CAPTURE_RSP) 校准捕获响应 [Size: 1 Byte] */
typedef struct {
    /** 捕获结果 (0=成功 ACCEPTED)。 */
    uint8_t response;
} mpp_tx_cal_capture_rsp_t;

/* Header: 0x1B - Calibration Operation Response (CAL_OP_RSP) 校准操作响应 [Size: 1 Byte] */
typedef struct {
    /** 操作执行状态结果。 */
    uint8_t status;
} mpp_tx_cal_op_rsp_t;

/* Header: 0x1E (Selector = 0x00) - Cloak Response 休眠响应 [Size: 1 Byte] */
typedef struct {
    /** 必须为 0x00。 */
    uint8_t selector;
} mpp_tx_cloak_t;

/* Header: 0x1E (Selector = 0x03) - Regulation Control Status 调节控制状态 [Size: 1 Byte] */
typedef struct {
    /** 必须为 0x03。 */
    uint8_t selector;
} mpp_tx_rcs_t;

/* Header: 0x1F - Charge Status (CHS) PTx充电状态 [Size: 1 Byte]
 * Figure 137 / Table 80 — battery-equipped PTx reports charge level to PRx.
 */
typedef struct {
    /** Charge Status Value: 0–100 = %; 0xFE = temporarily unavailable; 0xFF = no battery; else reserved. */
    uint8_t charge_status;
} mpp_tx_chs_t;

/* Header: 0x23 - Mode Select Status (MSS) 模式选择状态 [Size: 2 Bytes] */
typedef struct {
    /** [b7-b2]: Rsvd, [b1-b0]: Status (0=Success, 1=Pending, 2=Fail, 3=Busy)。 */
    uint8_t status_info;
    /** [b7-b4]: Rsvd, [b3-b0]: Error Code (例如 1=Not Supported)。 */
    uint8_t error_code;
} mpp_tx_mss_t;

/* Header: 0x2E - Get Request (GET) PTx主动获取请求 [Size: 2 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t rsvd;
    /** 请求从 PRx 获取的数据包代号。 */
    uint8_t parameter;
} mpp_tx_get_t;

/* Header: 0x2F - Enabled Data Streams (EDS) PTx支持流位掩码 [Size: 2 Bytes] */
typedef struct {
    /** 16位数据流位掩码 (大端模式)。 */
    uint16_t streams_bitmask;
} mpp_tx_eds_t;

/* Header: 0x34 - Enter Calibration Response (CAL_ENTER_RSP) 进入校准阶段响应 [Size: 3 Bytes] */
typedef struct {
    /** 0=Reject(拒绝), 1=Accept(同意)。 */
    uint8_t response_code;
    /** 拒绝原因 (例如 3=FO_DETECTED)。 */
    uint8_t reason;
    /** 附加校准时间参数。 */
    uint8_t parameter;
} mpp_tx_cal_enter_rsp_t;

/* Header: 0x3F (Selector = 0x00) - Inverter Voltage (INV) 逆变器电压 [Size: 3 Bytes] */
typedef struct {
    /** 必须为 0x00。 */
    uint8_t selector;
    /** 保留。 */
    uint8_t rsvd;
    /** PTx 测量的逆变电压。单位: 2 mV。(例如 4500=9.0V)。 */
    uint8_t v_inv;
} mpp_tx_inv_t;

/* Header: 0x3F (Selector = 0x01) - Simultaneous Data Stream Response PTx流响应 [Size: 3 Bytes] */
typedef struct {
    /** 必须为 0x01。 */
    uint8_t selector_rsvd;
    /** [b7-b4]: Rsvd, [b3-b0]: Stream Number (流通道号)。 */
    uint8_t stream_number;
    /** [b7-b4]: Rsvd, [b3-b0]: Type (0=ACK, 1=UNEXPECTED, 2=ERR_BUSY, 3=ERR_CRC)。 */
    uint8_t type_cmd;
} mpp_tx_sdsr_t;

/* Header: 0x3F (Selector = 0x02) - Estimated K (KEST) 估算耦合系数 [Size: 3 Bytes] */
typedef struct {
    /** 必须为 0x02。 */
    uint8_t selector;
    /** 保留。 */
    uint8_t rsvd;
    /** PTx 测算出的耦合系数。真实值 K = 字段值 / 4095。 */
    uint8_t estimated_k;
} mpp_tx_kest_t;

/* Header: 0x40 - Mated-Q Results (MATEDQ_RES) 配对Q值判定结果 [Size: 4 Bytes] */
typedef struct {
    /** [b7-b3]: Rsvd, [b2-b0]: Foreign Object (0=无法计算, 1=安全, 2=危险, 3=不确定)。 */
    uint8_t result;
    /** 3字节保留位数组，填充满 4 Bytes。 */
    uint8_t rsvd[1];
} mpp_tx_matedq_res_t;

/* Header: 0x43 - Calibration Capabilities (CAL_CAP) PTx校准能力 [Size: 4 Bytes] */
typedef struct {
    /** 32位 PTx 校准支持能力掩码 (大端模式)。 */
    uint32_t capabilities;
} mpp_tx_cal_cap_t;

/* Header: 0x4F - Simultaneous Auxiliary Data Control (SADC) PTx并发流控制 [Size: 4 Bytes]
 * Figure 147 / Table 63. Power Receiver shall respond with SDSR (0x38).
 */
typedef struct {
    /** [b7-b3]: Rsvd(=0), [b2-b0]: Request (Table 63). */
    uint8_t request;
    /** [b7-b4]: Rsvd(=0), [b3-b0]: Stream Number (Table 30). */
    uint8_t stream_number;
    /** 16-bit Parameter (Table 64), big-endian. */
    uint16_t parameter;
} mpp_tx_sadc_t;

/* Header: 0x54 - Calibration Parameter (dPCAL_PARAM) PTx校准参数反馈 [Size: 5 Bytes] */
typedef struct {
    /** [b7-b1]: Rsvd, [b0]: Invalid 标志位 (1=参数全部无效)。 */
    uint8_t status_flags;
    /** 拟合出的 DPLOSS Alpha 参数。16位大端。 */
    uint16_t dploss_alpha;
    /** 拟合出的 DPLOSS Beta 参数。16位大端。 */
    uint16_t dploss_beta;
} mpp_tx_dpcal_param_t;

/* Header: 0x5A - Power Modes Capabilities (MODECAP) 支持功率模式 [Size: 5 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t reserved;
    /** 位掩码。Bit0=CPM, Bit1=NPM, Bit2=LPM, Bit3=HPM。1代表支持该模式。 */
    uint8_t capabilities;
    /** 3字节保留位数组填充以满 5 Bytes。 */
    uint8_t rsvd_pad[1];
} mpp_tx_modecap_t;

/* Header: 0x5F - Power Loss Accounting Parameters (PLAP) 发射端基准参数 [Size: 5 Bytes] */
typedef struct {
    /** 保留。 */
    uint8_t rsvd;
    /** PTx 自身的线圈电阻损耗系数。16位大端有符号 (补码)。 */
    int16_t g_coil_r;
    /** 2字节保留位数组填充满 5 Bytes。 */
    uint8_t rsvd2[4];
} mpp_tx_plap_t;

/* Header: 0x61 - Gain Measurement Parameters (GMP) 增益测量参数 [Size: 6 Bytes] */
typedef struct {
    /** NPM模式增益测量常数 C0。16位大端。 */
    uint16_t g_npm_c0;
    /** HPM模式增益测量常数 C0。16位大端。 */
    uint16_t g_hpm_c0;
    /** CPM模式增益测量常数 C0。16位大端。 */
    uint16_t g_cpm_c0;
} mpp_tx_gmp_t;

/* Header: 0x8F:0 - Extended PTx Identification (XID) 发射端扩展ID [Size: 9 Bytes] */
typedef struct {
    /** 必须为 0x00。 */
    uint8_t selector;
    /** [b7-b1]: Rsvd, [b0]: APP 标志位 (MPP中固定为0)。 */
    uint8_t app_flag;
    /** 7 Bytes 设备 UID 数组。 */
    uint8_t device_id[6];
} mpp_tx_xid_t;

/* Header: 0x8F:1 - Extended PTx Capabilities (ECAP) 发射端扩展能力 [Size: 9 Bytes] */
typedef struct {
    /** 必须为 0x01。 */
    uint8_t selector;
    /** 8 Bytes 能力属性负载定长数组。 */
    uint8_t payload[3];
} mpp_tx_ecap_t;

/* ========================================================================= *
 * Header: 0xA0 (12 Bytes) - Power Modes Extended Capabilities (MODEXCAP)*
 * ========================================================================= */

/**
 * @brief 单个功率模式的能力参数包。每组精准占据 3 Bytes。
 */
typedef struct {
    /** 模式特定电压参考 Ref0。 */
    uint8_t v_ref0;
    /** 模式特定电压参考 Ref1。 */
    uint8_t v_ref1;
    /** 潜在最大负载功率 (单位: 100mW)。 */
    uint8_t potential_load_power;
} mpp_power_mode_params_t;

/**
 * @brief MODEXCAP (0xA0) 数据包载荷。4个模式 x 3 Bytes = 12 Bytes 完美对齐。
 */
typedef struct {
    /** 连续功率模式 (CPM) 参数。 */
    mpp_power_mode_params_t cpm_params;
    /** 低功率模式 (LPM) 参数。 */
    mpp_power_mode_params_t lpm_params;
    /** 正常功率模式 (NPM) 参数。 */
    mpp_power_mode_params_t npm_params;
    /** 高功率模式 (HPM) 参数。 */
    mpp_power_mode_params_t hpm_params;
} mpp_tx_modexcap_t;


/* ========================================================================= *
 *        3. DYNAMIC SIZED PACKETS (ADT / PROP) FOR BOTH DIRECTIONS          *
 * ========================================================================= */
/**
 * @brief 用于双向变长数据包 (如 S-ADT, PROP) 等根据 Header 计算长度的截取口
 * PRx: 0x1A, 0x1B, 0x2A, 0x2B, 0x26, 0x36 ...
 * PTx: 0x1C, 0x1D, 0x2C, 0x2D, 0x3E, 0x4E ...
 */
typedef struct {
    /** 变长数组的起始指针，通过 GET_PAYLOAD_LEN 宏计算具体长度。 */
    uint8_t dynamic_payload_start; 
} mpp_dynamic_packet_t;

#pragma pack(pop)

#ifdef __cplusplus
}
#endif

#endif // MPP_PROTOCOL_H