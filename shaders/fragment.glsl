#version 100

precision mediump float;

uniform sampler2D uTexture;

varying vec2 vTextureCoord;

void main()
{
    // Transform vertex to texture coordinates.
    vec2 coord = vec2(0.5 * vTextureCoord.x + 0.5, -0.5 * vTextureCoord.y + 0.5);
    gl_FragColor = texture2D(uTexture, coord);
}
