import { Connection } from "@solana/web3.js";

/**
 * A single shared Connection, deliberately configured to point at a different RPC endpoint than
 * whatever the backend uses for its own build-time simulation (see README's environment
 * variables) -- the whole point of simulating again client-side is that it should not share a
 * single point of failure or a single liar with the backend's own simulation result.
 */
let connection: Connection | null = null;

export function getConnection(): Connection {
  if (!connection) {
    const rpcUrl = import.meta.env.VITE_SOLANA_RPC_URL;
    if (!rpcUrl) {
      throw new Error(
        "VITE_SOLANA_RPC_URL is not configured -- see README.md's environment variable list",
      );
    }
    connection = new Connection(rpcUrl, "confirmed");
  }
  return connection;
}
