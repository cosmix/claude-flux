interface Point {
  x: number;
  y: number;
}

// Sample the SVG itself so routing regressions cannot hide behind unused data.
export function samplePath(path: string): Point[] {
  const samples: Point[] = [];
  let start = { x: 0, y: 0 };
  for (const command of path.match(/[MLC][^MLC]*/g) ?? []) {
    const values = command
      .slice(1)
      .trim()
      .split(/[\s,]+/)
      .map(Number);
    const end = { x: values.at(-2)!, y: values.at(-1)! };
    if (command[0] === "M") {
      samples.push(end);
    } else {
      for (let step = 0; step <= 200; step++) {
        const t = step / 200;
        const u = 1 - t;
        samples.push(
          command[0] === "C"
            ? {
                x:
                  u ** 3 * start.x +
                  3 * u ** 2 * t * values[0]! +
                  3 * u * t ** 2 * values[2]! +
                  t ** 3 * end.x,
                y:
                  u ** 3 * start.y +
                  3 * u ** 2 * t * values[1]! +
                  3 * u * t ** 2 * values[3]! +
                  t ** 3 * end.y,
              }
            : { x: start.x + t * (end.x - start.x), y: start.y + t * (end.y - start.y) },
        );
      }
    }
    start = end;
  }
  return samples;
}
