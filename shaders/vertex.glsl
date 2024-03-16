#version 100

attribute vec2 aVertexPosition;

uniform vec2 uPosition;
uniform mat2 uMatrix;

varying vec2 vTextureCoord;

void main()
{
    // Move texture anchor source to top-left of the texture.
    vec2 vertexPosition = aVertexPosition + vec2(1., -1.);

    // Apply texture size transform and move texture anchor target to top-left of the screen.
    vertexPosition = (uMatrix * vertexPosition) + vec2(-1., 1.);

    gl_Position = vec4(vertexPosition + uPosition, 0., 1.);

    vTextureCoord = aVertexPosition;
}
