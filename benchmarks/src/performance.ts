/** Action-window lab measurements. These deliberately do not call themselves INP. */
export const beginActionSampling = `(() => {
  if (window.__mochiActionSample) throw Error('Action sampling namespace is occupied');
  const sample = {started: performance.now(), last: performance.now(), intervals: [], frame: 0, capped: false};
  const tick = now => {
    sample.intervals.push(now - sample.last); sample.last = now;
    if (sample.intervals.length < 600 && now - sample.started < 15000) sample.frame = requestAnimationFrame(tick);
    else sample.capped = true;
  };
  sample.frame = requestAnimationFrame(tick); window.__mochiActionSample = sample;
  return true;
})()`;
export const endActionSampling = `(() => {
  const sample = window.__mochiActionSample;
  if (!sample) return {intervals: [], elapsedMs: null, capped: false, navigationInterrupted: true, scope: 'Document navigation reset the sampler; action frame timing is unmeasured'};
  cancelAnimationFrame(sample.frame); delete window.__mochiActionSample;
  return {intervals: sample.intervals, elapsedMs: performance.now() - sample.started, capped: sample.capped,
    scope: 'Frames across declared actions and settling; includes browser-driver overhead, not input latency or field INP'};
})()`;
