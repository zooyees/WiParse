/** Direct HTTP client for WiParse GUI embedded API (C+E). */
export interface ApiEnvelope {
    ok: boolean;
    cmd?: string;
    data?: unknown;
    error?: {
        code?: string;
        message: string;
    };
    ts?: string;
}
export declare function defaultApiUrl(): string;
export declare function apiHealth(baseUrl?: string): Promise<ApiEnvelope>;
export declare function apiCapabilities(baseUrl?: string): Promise<ApiEnvelope>;
export declare function apiInvoke(method: string, params?: unknown, baseUrl?: string, timeoutMs?: number): Promise<ApiEnvelope>;
