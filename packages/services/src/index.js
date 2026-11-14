export function readTextAsset(path) {
  return globalThis.__zguiServices.readTextAsset(path);
}

export function watchNiri(onSnapshot, onError) {
  return globalThis.__zguiServices.watchNiri(onSnapshot, onError);
}
