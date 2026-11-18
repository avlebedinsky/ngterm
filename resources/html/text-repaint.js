// ngterm's own fix over upstream's text.js, which runs after it in the same world. WebKit paints a ::highlight from
// the bottom of the line above, but when a highlight changes, it repaints only the text boxes in its ranges. When
// escape sequences split the text into many text nodes, the line gap above an old match keeps its color. A changed,
// transparent background on the text block repaints all of the block once after each clear.
(() => {
  if (typeof Highlight !== 'function') return;
  const clear = Highlight.prototype.clear;
  let queued = false;
  let flip = false;
  Highlight.prototype.clear = function () {
    clear.call(this);
    if (queued) return;
    queued = true;
    queueMicrotask(() => {
      queued = false;
      const pre = document.querySelector('#ngterm-page pre');
      if (!pre) return;
      flip = !flip;
      pre.style.backgroundColor = flip ? 'rgba(255, 255, 255, 0)' : 'rgba(0, 0, 0, 0)';
    });
  };
})();
