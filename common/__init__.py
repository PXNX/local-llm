from . import storage

# every flow imports common first: model downloads (rembg, transformers, torch hub) land in MODELS_DIR
storage.route()
