/**
 * @file    bpp_protocol.h
 * @brief   WPC Qi Standard (BPP/EPP) Protocol Data Packet Structures
 * @version 2.7 (Ultimate Edition: Strict Alignment, Full Headers, Deep Comments)
 * @note    所有多字节字段 (uint16_t, uint32_t) 在空中均以大端模式 (BIG-ENDIAN) 传输。
 *          小端架构的 MCU 在解析时必须使用 ntohs() / ntohl() 进行字节序转换。
 */

#ifndef BPP_PROTOCOL_H
#define BPP_PROTOCOL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= *
 *                       PAYLOAD SIZE CALCULATION                            *
 * ========================================================================= */
/**
 * @brief 根据 Header 字节计算有效载荷 (Payload) 的长度（N Bytes）。
 *        计算结果向下取整，适用于 ASK 和 FSK 数据包。
 */
#define GET_PAYLOAD_LEN(header) \
    ((header) <= 0x1F ? (1 + ((header) / 32)) : \
    ((header) <= 0x7F ? (2 + ((header) - 32) / 16) : \
    ((header) <= 0xDF ? (8 + ((header) - 128) / 8) : \
                        (20 + ((header) - 224) / 4))))

#pragma pack(push, 1)

/* ========================================================================= *
 *             DIRECTION 1: PRX -> PTX (ASK MODULATION MESSAGES)             *
 * ========================================================================= */

/* Header: 0x01 - Signal Strength (SIG) 信号强度 [Size: 1 Byte] */
typedef struct {
    /** 信号强度值。公式: (U / U_max) * 256。取值范围: 0-255。 */
    uint8_t signal_strength;
} bpp_rx_sig_t;

/* Header: 0x02 - End Power Transfer (EPT) 结束功率传输 [Size: 1 Byte] */
typedef struct {
    /** 结束原因代码: 
     *  0x00=未知(nul), 0x01=充电完成(cc), 0x02=内部故障(if), 0x03=过温(ot), 
     *  0x04=过压(ov), 0x05=过流(oc), 0x06=电池故障(bf), 0x08=无响应(nr), 
     *  0x0A=协商失败(an), 0x0B=请求重启(rst), 0x0C=重新Ping(rep), 0x0D=NFC(nfc) */
    uint8_t reason_code;
} bpp_rx_ept_t;

/* Header: 0x03 - Control Error (CE) 控制误差 [Size: 1 Byte] */
typedef struct {
    /** 控制误差值。8位有符号整数(补码)。正值要求增加功率，负值要求减少功率。 */
    int8_t control_error;
} bpp_rx_ce_t;

/* Header: 0x04 - Received Power 8-bit (RP8) 8位接收功率 [Size: 1 Byte] */
typedef struct {
    /** 预估的接收功率电平。公式: (预估功率 / 最大功率) * 128。 */
    uint8_t received_power;
} bpp_rx_rp8_t;

/* Header: 0x05 - Charge Status (CHS) 充电状态 [Size: 1 Byte] */
typedef struct {
    /** 电池电量百分比。取值: 0 到 100。0xFF (255) 表示状态不可用。 */
    uint8_t charge_status;
} bpp_rx_chs_t;

/* Header: 0x06 - Power Control Hold-off (PCH) 功率控制延时 [Size: 1 Byte] */
typedef struct {
    /** 延时时间，单位：毫秒 (ms)。合法范围：5 ms 到 100 ms。 */
    uint8_t hold_off_time;
} bpp_rx_pch_t;

/* Header: 0x07 - General Request (GRQ) 通用请求 [Size: 1 Byte] */
typedef struct {
    /** 请求PTx回复的特定包Header。如: 0x31 请求 CAP，0x32 请求 XCAP。 */
    uint8_t req_header;
} bpp_rx_grq_t;

/* Header: 0x09 - Renegotiate (NEGO) 重新协商 [Size: 1 Byte] */
typedef struct {
    /** 必须为 0x00。触发重新进入协商阶段。 */
    uint8_t reserved;
} bpp_rx_nego_t;

/* Header: 0x15 - Data Stream Response (DSR) 数据流响应 [Size: 1 Byte] */
typedef struct {
    /** 响应代码: 0x00=ACK, 0x01=POLL, 0x02=NAK, 0x03=ND。 */
    uint8_t response_code;
} bpp_rx_dsr_t;

/* Header: 0x20 - Specific Request (SRQ) 特定请求 [Size: 2 Bytes] */
typedef struct {
    /** 请求代码: 0x00=结束协商, 0x01=保证功率, 0x03=FSK配置, 0x05=Reping 等。 */
    uint8_t request_code;
    /** 与 request_code 强关联的具体参数。 */
    uint8_t parameter;
} bpp_rx_srq_t;

/* Header: 0x22 - FOD Status (FOD) 异物检测状态 [Size: 2 Bytes] */
typedef struct {
    /** [b7-b1]: Rsvd, [b0]: FOD类型 (0 = Q-Factor, 1 = 共振频率)。 */
    uint8_t type_info;
    /** 参考品质因数(Q) 或 参考频率。 */
    uint8_t support_data;
} bpp_rx_fod_t;

/* Header: 0x25 - Auxiliary Data Control (ADC) 辅助数据控制 [Size: 2 Bytes] */
typedef struct {
    /** 请求动作代码: 0x10=认证(Auth), 0x28=复位(Reset) 等。 */
    uint8_t request;
    /** 与请求动作关联的参数。 */
    uint8_t parameter;
} bpp_rx_adc_t;

/* Header: 0x31 - Received Power 16-bit (RP) 16位接收功率 [Size: 3 Bytes] */
typedef struct {
    /** [b7-b3]: Rsvd, [b2-b0]: 测量模式 (0=Default, 1=In-band, 2=Out-of-band)。 */
    uint8_t mode_info;
    /** 16位预估接收功率值。单位 1 mW (大端模式)。 */
    uint16_t rx_power;
} bpp_rx_rp_t;

/* Header: 0x51 - Configuration (CFG) 配置包 [Size: 5 Bytes] */
typedef struct {
    /** [b7-b6]: 00, [b5-b0]: Reference Power (最大值10代表5W)。 */
    uint8_t ref_power;
    /** 必须为 0x00。 */
    uint8_t rsvd1;
    /** [b6]: AI (支持鉴权), [b4]: OB (带外通信), [b2-b0]: 可选包Count。 */
    uint8_t features;
    /** [b7-b3]: Window Size, [b2-b0]: Window Offset。 */
    uint8_t window_cfg;
    /** [b7]: Neg (1=EPP), [b6]: Pol, [b5-b4]: Depth, [b3-b1]: Buffer Size, [b0]: Dup。 */
    uint8_t fsk_cfg;
} bpp_rx_cfg_t;

/* Header: 0x54 (hi) & 0x55 (lo) - Wireless Power ID (WPID) 无线功率识别 [Size: 5 Bytes] */
typedef struct {
    /** WPID 分段数据。严格定义为长度 3 的数组以保证 5 Byte 总长对齐。 */
    uint8_t wpid_segment[1]; 
    /** 16位 CRC 校验和 (大端)。 */
    uint16_t crc;
} bpp_rx_wpid_t;

/* Header: 0x71 - Identification (ID) 身份识别包 [Size: 7 Bytes] */
typedef struct {
    /** [b7-b4]: Major Version, [b3-b0]: Minor Version。 */
    uint8_t version;
    /** 16位制造商代码 (大端)。 */
    uint16_t mfg_code;
    /** [b31]: Ext (1=有XID包), [b30-b0]: Basic Device ID (大端)。 */
    uint32_t basic_dev_id;
} bpp_rx_id_t;

/* Header: 0x81 - Extended Identification (XID) 扩展身份识别包 [Size: 8 Bytes] */
typedef struct {
    /** BPP/EPP 中纯 8 字节的扩展ID。严格定义为长度 8 的数组。(第一字节不可为0xFE) */
    uint8_t ext_device_id[3];  
} bpp_rx_xid_t;


/* ========================================================================= *
 *             DIRECTION 2: PTX -> PRX (FSK MODULATION MESSAGES)             *
 * ========================================================================= */

/* FSK 底层响应模式 (无 Header 和 Checksum) */
typedef enum {
    BPP_FSK_ACK = 0x55,  /**< 确认 (01010101) */
    BPP_FSK_NAK = 0x00,  /**< 拒绝 (00000000) */
    BPP_FSK_ND  = 0xAA,  /**< 未定义/不支持 (10101010) */
    BPP_FSK_ATN = 0x0F   /**< 注意/请求通信 (00001111) */
} bpp_tx_fsk_pattern_t;

/* Header: 0x00 - Data Not Available (NULL) 空数据包 [Size: 1 Byte] */
typedef struct {
    /** 必须固定为 0x00。 */
    uint8_t reserved;
} bpp_tx_null_t;

/* Header: 0x15 - Data Stream Response (DSR PTx) PTx数据流响应 [Size: 1 Byte] */
typedef struct {
    /** 0x00=ACK, 0x01=POLL, 0x02=NAK, 0x03=ND。 */
    uint8_t response_code;
} bpp_tx_dsr_t;

/* Header: 0x25 - Auxiliary Data Control (ADC PTx) PTx辅助数据控制 [Size: 2 Bytes] */
typedef struct {
    /** PTx 发起的请求动作。 */
    uint8_t request;    
    /** 参数。 */
    uint8_t parameter;  
} bpp_tx_adc_t;

/* Header: 0x30 - Power Transmitter Identification (ID) PTx识别包 [Size: 3 Bytes] */
typedef struct {
    /** PTx 的制造商与身份代码信息载荷。严格使用 3 字节数组。 */
    uint8_t ptx_id_payload[1];
} bpp_tx_id_t;

/* Header: 0x31 - Power Transmitter Capabilities (CAP) 发射端能力包 [Size: 3 Bytes] */
typedef struct {
    /** [b7-b6]: Power Class (0=Class 0), [b5-b0]: Guaranteed Power。 */
    uint8_t power_info;
    /** 必须为 0x00。 */
    uint8_t reserved;
    /** 潜在负载功率 (如 10表示5W，30表示15W)。 */
    uint8_t potential_power;
} bpp_tx_cap_t;

/* Header: 0x32 - Extended Capabilities (XCAP) 发射端扩展能力包 [Size: 3 Bytes] */
typedef struct {
    /** [b7]: TPS (带温度保护), [b6]: TDE (热降额), [b5]: TDS (热数据流)。 */
    uint8_t capabilities;
    /** 必须为 0x00。 */
    uint8_t reserved_1;
    /** 必须为 0x00。 */
    uint8_t reserved_2;
} bpp_tx_xcap_t;


/* ========================================================================= *
 *        3. DYNAMIC SIZED PACKETS (ADT / PROP) FOR BOTH DIRECTIONS          *
 * ========================================================================= */
/**
 * @brief 用于 PRx 和 PTx 双方变长的数据流透传 (ADT) 及私有包 (PROP)。
 * 包含的 Headers: 
 * - ADT: 0x16, 0x17, 0x26, 0x27, 0x36, 0x37, 0x46, 0x47, 0x56, 0x57, 0x66, 0x67, 0x76, 0x77
 * - PTx 专有扩展 ADT: 0x98, 0x99 (ADT/11e, ADT/11o)
 * - PROP: 0x18, 0x19, 0x28, 0x29 ... 0x7F, 0x84, 0xA4, 0xC4, 0xE2 等等
 * 
 * 软件使用指南: 根据 GET_PAYLOAD_LEN 截断实际字节数。
 */
typedef struct {
    /** 变长数组的起始指针，通过宏计算具体长度。 */
    uint8_t dynamic_payload_start; 
} bpp_dynamic_packet_t;

#pragma pack(pop)

#ifdef __cplusplus
}
#endif

#endif // BPP_PROTOCOL_H
