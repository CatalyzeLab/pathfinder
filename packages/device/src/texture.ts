export class WebGlTexture {
  static create (
    context: WebGl2RenderingContext, 
    texture: WebGlTexture,
    format: TextureFormat,
    size: number
  ) {
    return new WebGlTexture(context, texture, format, size)
  }

  public get backend () {
    return 'WebGL'
  }

  public get device () {
    return 'WebGL Device'
  }

  public context: WebGl2RenderingContext
  public glTexture: WebGLTexture
  public format: TextureFormat
  public size: number

  constructor (
    context: WebGl2RenderingContext, 
    glTexture: WebGLTexture,
    format: TextureFormat,
    size: number
  ) {
    this.context = context
    this.glTexture = glTexture
    this.format = format
    this.size = this.size
  }

  delete () {
    this.context.deleteTexture(this.glTexture)
  }
}