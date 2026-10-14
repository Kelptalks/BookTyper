(() => {
  'use strict';
  const text = 'Somewhere between the first word and the next page, you find your rhythm.';
  const field = document.querySelector('#typing-input');
  const passage = document.querySelector('#passage');
  const hint = document.querySelector('#typing-hint');
  const progress = document.querySelector('#demo-progress');
  const fill = document.querySelector('#progress-fill');
  const frame = document.querySelector('.demo-frame');
  const canvas = document.querySelector('#web-canvas');
  const ctx = canvas.getContext('2d');
  const motion = document.querySelector('#motion-toggle');
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  let paused = reduced.matches;
  let atmosphere = 'web';
  let visible = true;
  let w = 600, h = 460, dpr = 1;
  let offset = 0, targetOffset = 0, activity = 0, wave = -1;
  let previousValue = '';
  let previousTime = 0;
  let raf = 0;
  let seed = 83724;
  const random = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed / 4294967296;
  };
  const letters = [...text].map(char => {
    const span = document.createElement('span');
    span.textContent = char;
    passage.append(span);
    return span;
  });
  document.querySelector('label[for="typing-input"]').textContent = `Type this passage: ${text}`;
  field.maxLength = text.length;

  function updateText() {
    const value = field.value.replace(/[\r\n]/g, '').slice(0, text.length);
    if (field.value !== value) field.value = value;
    let correct = 0;
    letters.forEach((span, i) => {
      const typed = i < value.length;
      const match = value[i] === text[i];
      span.className = typed ? (match ? 'typed' : 'wrong') : (i === value.length ? 'cursor' : '');
      if (typed && match) correct++;
    });
    progress.textContent = `${correct} / ${text.length} characters`;
    fill.style.width = `${correct / text.length * 100}%`;
    targetOffset = value.length * 3;
    if (value.length > previousValue.length) activity = Math.min(1, activity + 0.2);
    if (value === text) {
      hint.textContent = 'There it is. One small page, all yours.';
      if (previousValue !== text) wave = 0;
    } else if (value.length && correct !== value.length) {
      hint.textContent = 'A small stumble. Backspace to correct the warm-colored letters.';
    } else {
      hint.textContent = value.length ? 'Keep going. The page moves with you.' : 'Click the text. Type the words. Take your time.';
    }
    previousValue = value;
    if (paused || atmosphere === 'quiet') draw(0);
    startAnimation();
  }

  field.addEventListener('input', updateText);
  field.addEventListener('keydown', e => { if (e.key === 'Enter') e.preventDefault(); });
  passage.addEventListener('click', () => field.focus());
  document.querySelector('#reset-demo').addEventListener('click', () => {
    field.value = '';
    previousValue = '';
    wave = -1;
    updateText();
    field.focus({ preventScroll: true });
  });
  document.querySelectorAll('[data-atmosphere]').forEach(button => {
    button.addEventListener('click', () => {
      atmosphere = button.dataset.atmosphere;
      frame.dataset.theme = atmosphere;
      document.querySelectorAll('[data-atmosphere]').forEach(b => b.setAttribute('aria-pressed', String(b === button)));
      draw(0);
      startAnimation();
    });
  });
  function updateMotionButton() {
    motion.setAttribute('aria-pressed', String(paused));
    motion.setAttribute('aria-label', paused ? 'Play background animation' : 'Pause background animation');
    motion.innerHTML = paused
      ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 4 12 8-12 8Z"/></svg>'
      : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 5v14M15 5v14"/></svg>';
  }
  motion.addEventListener('click', () => {
    paused = !paused;
    updateMotionButton();
    draw(0);
    startAnimation();
  });
  reduced.addEventListener('change', e => {
    paused = e.matches;
    updateMotionButton();
    draw(0);
    startAnimation();
  });

  const meshes = [];
  const side = 700;
  for (let layer = 0; layer < 3; layer++) {
    const surface = document.createElement('canvas');
    surface.width = side;
    surface.height = side;
    const c = surface.getContext('2d');
    const nodes = [];
    const edges = [];
    const cols = 11, rows = 13;
    for (let row = 0; row <= rows; row++) {
      for (let col = 0; col <= cols; col++) {
        nodes.push({ x: col * side / cols + (random() - 0.5) * 38, y: row * side / rows + (random() - 0.5) * 36, links: [] });
      }
    }
    function connect(a, b) {
      const edge = { a, b };
      edges.push(edge);
      nodes[a].links.push(b);
      nodes[b].links.push(a);
    }
    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < cols; col++) {
        const a = row * (cols + 1) + col;
        if (random() > 0.12) connect(a, a + 1);
        if (random() > 0.12) connect(a, a + cols + 1);
        if (random() < 0.25) connect(a, a + cols + 2);
      }
    }
    c.strokeStyle = ['#8eb79d', '#6f9582', '#527263'][layer];
    c.lineWidth = [1.1, 0.8, 0.65][layer];
    c.lineCap = 'round';
    edges.forEach(edge => {
      const a = nodes[edge.a], b = nodes[edge.b];
      c.globalAlpha = 0.28 + random() * 0.45;
      c.beginPath();
      c.moveTo(a.x, a.y);
      c.lineTo(b.x, b.y);
      c.stroke();
    });
    const sparks = Array.from({ length: layer === 0 ? 15 : 8 }, () => {
      const e = edges[Math.floor(random() * edges.length)];
      return { a: e.a, b: e.b, t: random(), speed: 12 + random() * 15 };
    });
    meshes.push({ surface, nodes, edges, sparks });
  }
  const stars = Array.from({ length: 95 }, () => ({ x: (random() - 0.5) * 2, y: (random() - 0.5) * 2, z: random() + 0.05 }));

  function draw(dt) {
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = atmosphere === 'streaks' ? '#111721' : '#101b15';
    ctx.fillRect(0, 0, w, h);
    const moving = !paused && visible;
    const delta = moving ? dt : 0;
    offset += (targetOffset - offset) * (paused ? 1 : Math.min(1, 0.06 + delta * 5));
    activity *= Math.exp(-delta * 2.2);
    if (atmosphere === 'web') {
      meshes.forEach((mesh, index) => {
        const scale = (w / side) * (1.35 - index * 0.12);
        const tileH = side * scale;
        const scroll = (offset / (index + 1)) % tileH;
        ctx.save();
        ctx.globalAlpha = [0.65, 0.4, 0.24][index];
        for (let y = -tileH; y < h + tileH; y += tileH) ctx.drawImage(mesh.surface, -20 + index * 40, y - scroll, side * scale, tileH);
        ctx.restore();
        mesh.sparks.forEach(spark => {
          const a = mesh.nodes[spark.a], b = mesh.nodes[spark.b];
          const length = Math.hypot(b.x - a.x, b.y - a.y) || 1;
          spark.t += delta * (spark.speed + activity * 24) / length;
          if (spark.t > 1) {
            const options = b.links.filter(n => n !== spark.a);
            const next = options.length ? options[Math.floor(random() * options.length)] : spark.a;
            spark.a = spark.b;
            spark.b = next;
            spark.t = 0;
          }
          const from = mesh.nodes[spark.a], to = mesh.nodes[spark.b];
          const x = (from.x + (to.x - from.x) * spark.t) * scale - 20 + index * 40;
          const baseY = (from.y + (to.y - from.y) * spark.t) * scale - scroll;
          for (let shift = -1; shift <= 1; shift++) {
            const y = baseY + shift * tileH;
            if (y < -15 || y > h + 15) continue;
            const glow = ctx.createRadialGradient(x, y, 0, x, y, 11);
            glow.addColorStop(0, index === 0 ? '#d7ebc5ba' : '#afcfbb70');
            glow.addColorStop(.18, '#badbb96a');
            glow.addColorStop(1, '#badbb900');
            ctx.fillStyle = glow;
            ctx.fillRect(x - 11, y - 11, 22, 22);
          }
        });
      });
    } else if (atmosphere === 'streaks') {
      const speed = 0.045 + activity * 0.4;
      stars.forEach(star => {
        star.z -= delta * speed;
        if (star.z <= 0.06) { star.z = 1; star.x = (random() - .5) * 2; star.y = (random() - .5) * 2; }
        const x = w / 2 + star.x * w * .3 / star.z;
        const y = h / 2 + star.y * h * .3 / star.z;
        const tail = .012 + activity * .045;
        ctx.beginPath();
        ctx.moveTo(x, y);
        ctx.lineTo(w / 2 + star.x * w * .3 / (star.z + tail), h / 2 + star.y * h * .3 / (star.z + tail));
        ctx.strokeStyle = `rgba(169,193,220,${(1 - star.z) * .6 + .15})`;
        ctx.lineWidth = Math.max(.6, 1.4 - star.z);
        ctx.stroke();
      });
    } else {
      const light = ctx.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, w * .8);
      light.addColorStop(0, '#28362b');
      light.addColorStop(1, '#111b15');
      ctx.fillStyle = light;
      ctx.fillRect(0, 0, w, h);
    }
    if (wave >= 0 && !paused) {
      wave += delta;
      ctx.beginPath();
      ctx.ellipse(w / 2, h / 2, wave * w * .5, wave * w * .5, 0, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(228,209,169,${Math.max(0, .8 - wave * .35)})`;
      ctx.lineWidth = 4;
      ctx.shadowColor = '#ccdfa7';
      ctx.shadowBlur = 20;
      ctx.stroke();
      ctx.shadowBlur = 0;
      if (wave > 2.4) wave = -1;
    }
  }
  function animate(now) {
    raf = 0;
    if (!visible || paused || atmosphere === 'quiet' || document.hidden) { previousTime = 0; return; }
    if (!previousTime || now - previousTime >= 1000 / 30) {
      const dt = previousTime ? Math.min((now - previousTime) / 1000, .06) : 0;
      previousTime = now;
      draw(dt);
    }
    raf = requestAnimationFrame(animate);
  }
  function startAnimation() { if (!raf && visible && !paused && atmosphere !== 'quiet' && !document.hidden) raf = requestAnimationFrame(animate); }
  function resize() {
    const rect = canvas.parentElement.getBoundingClientRect();
    w = rect.width;
    h = rect.height;
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    draw(0);
    startAnimation();
  }
  new ResizeObserver(resize).observe(canvas.parentElement);
  new IntersectionObserver(entries => { visible = entries[0].isIntersecting; startAnimation(); }, { threshold: 0 }).observe(canvas);
  document.addEventListener('visibilitychange', startAnimation);
  updateMotionButton();
  updateText();
  resize();
})();
