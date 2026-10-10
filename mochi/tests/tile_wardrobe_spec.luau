-- Tile spec: the wardrobe tile toggles its panel.
onClick()
assert(lastCall() == "toggle coucou/mochi:wardrobe", "wardrobe tile toggles the wardrobe panel")
print("TILE_WARDROBE PASSED")
