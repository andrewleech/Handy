import type { EngineType } from "@/bindings";

/** Engine types managed exclusively by the streaming subsystem. */
export const STREAMING_ENGINE_TYPES: EngineType[] = [
  "NemotronStreaming",
  "Qwen3Streaming",
];

/** Returns true if the engine type is streaming-only (not a batch transcription engine). */
export const isStreamingEngine = (engineType: EngineType): boolean =>
  STREAMING_ENGINE_TYPES.includes(engineType);
