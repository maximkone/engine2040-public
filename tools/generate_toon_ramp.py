#!/usr/bin/env python3
# Generate TOON1 toon_ramp.png: 256x1 gradient from black to white
import numpy as np
from PIL import Image

# Create a 256x1 gradient (0 to 255)
gradient = np.linspace(0, 255, 256, dtype=np.uint8)
# Create RGBA array: 256x1x4
ramp = np.zeros((256, 1, 4), dtype=np.uint8)
# Set RGB channels to gradient
ramp[:, :, 0] = gradient.reshape(-1, 1)  # R
ramp[:, :, 1] = gradient.reshape(-1, 1)  # G
ramp[:, :, 2] = gradient.reshape(-1, 1)  # B
ramp[:, :, 3] = 255  # A (fully opaque)

# Create image
img = Image.fromarray(ramp, 'RGBA')
img.save('content/design/toon_ramp.png')
print('TOON1 toon_ramp.png created: 256x1 gradient')