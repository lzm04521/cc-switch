export interface Point {
  x: number;
  y: number;
}

/** 位移小于阈值（默认 5px）判定为点击，否则视为拖拽 */
export function isClickGesture(down: Point, up: Point, threshold = 5): boolean {
  return Math.hypot(up.x - down.x, up.y - down.y) < threshold;
}
