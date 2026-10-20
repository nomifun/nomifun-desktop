// Happy DOM currently omits Document.compatMode. Establish the real renderer's
// standards document without replacing the body captured by Testing Library.
if (!document.doctype) document.insertBefore(document.implementation.createDocumentType('html', '', ''), document.documentElement);
if (document.compatMode === undefined) Object.defineProperty(document, 'compatMode', {
  configurable: true,
  get: () => document.doctype?.name === 'html' ? 'CSS1Compat' : 'BackCompat',
});
