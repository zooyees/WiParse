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
export declare function isLocalBridge(baseUrl: string): boolean;
export declare function localHeaders(baseUrl: string, env?: NodeJS.ProcessEnv): Record<string, string>;
export declare function apiHealth(baseUrl?: string): Promise<ApiEnvelope>;
export declare function apiCapabilities(baseUrl?: string): Promise<ApiEnvelope>;
export declare function apiInvoke(method: string, params?: unknown, baseUrl?: string, timeoutMs?: number): Promise<ApiEnvelope>;
