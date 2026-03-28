// mandelbrot_bench.js

function mandelbrot(width, height, maxIter) {
  let count = 0;

  for (let y = 0; y < height; y++) {
    const cy = (y / height) * 3.0 - 1.5;

    for (let x = 0; x < width; x++) {
      const cx = (x / width) * 3.5 - 2.5;

      let zx = 0.0;
      let zy = 0.0;
      let iter = 0;

      while (zx * zx + zy * zy <= 4.0 && iter < maxIter) {
        const zx2 = zx * zx - zy * zy + cx;
        zy = 2.0 * zx * zy + cy;
        zx = zx2;
        iter++;
      }

      if (iter === maxIter) count++;
    }
  }

  return count;
}

function bench() {
  const width = 400;
  const height = 300;
  const maxIter = 200;

  const t0 = (typeof performance !== "undefined" ? performance.now() : Date.now());
  const inside = mandelbrot(width, height, maxIter);
  const t1 = (typeof performance !== "undefined" ? performance.now() : Date.now());

  const ms = t1 - t0;

  console.log("Mandelbrot benchmark");
  console.log("  size:", width + "x" + height);
  console.log("  maxIter:", maxIter);
  console.log("  inside:", inside);
  console.log("  time:", ms.toFixed(3), "ms");
}

bench();