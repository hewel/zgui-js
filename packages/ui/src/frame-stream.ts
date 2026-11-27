/** A session-owned stream of native textures; no pixel bytes enter JavaScript. */
export interface FrameStreamHandle {
  readonly kind: "frame-stream";
  readonly streamId: number;
  /** Opaque source for Image or the intrinsic image element. */
  readonly src: string;
  readonly width: number;
  readonly height: number;
  release(): void;
}
