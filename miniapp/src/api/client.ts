import { getInitData } from "../telegram/webapp";
import type {
  AddLiquidityRequest,
  BalancesResponse,
  BuildTxResponse,
  ClaimFeesRequest,
  ClosePositionRequest,
  OpenPositionRequest,
  PositionsResponse,
  RegisterWalletRequest,
  RegisterWalletResponse,
  RemoveLiquidityRequest,
  SubmitTxRequest,
  SubmitTxResponse,
  TxStatusResponse,
} from "./types";

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly code: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/**
 * initData rides on every request as a header, never in the body -- keeping it out of any
 * request/response shape the backend might log wholesale by accident, the same discipline the
 * README asks the backend's own request/response types to hold to for key material. The backend
 * is responsible for recomputing and checking it; this client never inspects or trusts it.
 */
const INIT_DATA_HEADER = "X-Telegram-Init-Data";

export interface ApiClientConfig {
  baseUrl: string;
}

export class ApiClient {
  constructor(private readonly config: ApiClientConfig) {}

  private async request<TResponse>(
    method: "GET" | "POST",
    path: string,
    body?: unknown,
  ): Promise<TResponse> {
    const init: RequestInit = {
      method,
      headers: {
        "Content-Type": "application/json",
        [INIT_DATA_HEADER]: getInitData(),
      },
    };
    if (body !== undefined) {
      init.body = JSON.stringify(body);
    }
    const res = await fetch(`${this.config.baseUrl}${path}`, init);

    if (!res.ok) {
      let code = "unknown";
      let message = `request failed with status ${res.status}`;
      try {
        const parsed = (await res.json()) as { error?: string; code?: string };
        if (parsed.error) message = parsed.error;
        if (parsed.code) code = parsed.code;
      } catch {
        // response body was not JSON; fall back to the generic message above
      }
      throw new ApiError(message, res.status, code);
    }

    return (await res.json()) as TResponse;
  }

  registerWallet(req: RegisterWalletRequest): Promise<RegisterWalletResponse> {
    return this.request("POST", "/api/v1/wallet/register", req);
  }

  getBalances(): Promise<BalancesResponse> {
    return this.request("GET", "/api/v1/wallet/balances");
  }

  getPositions(): Promise<PositionsResponse> {
    return this.request("GET", "/api/v1/positions");
  }

  buildOpenPosition(req: OpenPositionRequest): Promise<BuildTxResponse> {
    return this.request("POST", "/api/v1/tx/open-position", req);
  }

  buildAddLiquidity(req: AddLiquidityRequest): Promise<BuildTxResponse> {
    return this.request("POST", "/api/v1/tx/add-liquidity", req);
  }

  buildRemoveLiquidity(req: RemoveLiquidityRequest): Promise<BuildTxResponse> {
    return this.request("POST", "/api/v1/tx/remove-liquidity", req);
  }

  buildClaimFees(req: ClaimFeesRequest): Promise<BuildTxResponse> {
    return this.request("POST", "/api/v1/tx/claim-fees", req);
  }

  buildClosePosition(req: ClosePositionRequest): Promise<BuildTxResponse> {
    return this.request("POST", "/api/v1/tx/close-position", req);
  }

  /** Submits an already-signed transaction. The backend is expected to relay these raw signed
   * bytes to RPC opaquely, without inspecting or altering them -- see README for why a thin
   * relay was chosen over the Mini App holding its own paid RPC key. */
  submitTransaction(req: SubmitTxRequest): Promise<SubmitTxResponse> {
    return this.request("POST", "/api/v1/tx/submit", req);
  }

  getTransactionStatus(signature: string): Promise<TxStatusResponse> {
    return this.request("GET", `/api/v1/tx/status?signature=${encodeURIComponent(signature)}`);
  }
}
